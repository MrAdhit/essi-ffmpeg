use std::{env::{current_exe, temp_dir}, ffi::OsStr, fs::{File, OpenOptions}, io::{Cursor, Read, Write}, marker::PhantomData, ops::AddAssign, path::PathBuf, process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio}};

use anyhow::Context;
use flate2::read::GzDecoder;
use once_cell::sync::Lazy;
use pipe::{Pipe, Piped};
use rand::{distributions::Alphanumeric, Rng};
use tokio::{sync::mpsc::{channel, Receiver, Sender}, task::JoinHandle};

pub mod pipe;

/// https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.0
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const FFMPEG_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-win32-x64.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const FFMPEG_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-linux-x64.gz";

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const FFMPEG_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-linux-arm64.gz";

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const FFMPEG_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-x64.gz";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const FFMPEG_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-arm64.gz";

static mut FFMPEG_DOWNLOAD_ROOT_DIR: Lazy<PathBuf> = Lazy::new(|| current_exe().expect("Can't get the current app path").parent().clone().expect("Can't get the current program folder.\nThis should never fail... I think").to_path_buf());

#[derive(Debug)]
pub enum FFmpegProgressStatus {
    Continue,
    End,
}

impl std::str::FromStr for FFmpegProgressStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "continue" => Ok(Self::Continue),
            "end" => Ok(Self::End),
            _ => anyhow::bail!("Can't parse from {s:?}"),
        }
    }
}

#[derive(Debug, Default)]
pub struct FFmpegProgress
{
    pub frame: Option<usize>,
    pub fps: Option<usize>,
    pub bitrate: Option<f32>,
    pub total_size: Option<usize>,
    pub out_time_us: Option<usize>,
    pub out_time_ms: Option<usize>,
    pub dup_frames: Option<usize>,
    pub drop_frames: Option<usize>,
    pub speed: Option<f32>,
    pub progress: Option<FFmpegProgressStatus>,
}

impl From<String> for FFmpegProgress {
    fn from(progress: String) -> Self {
        let ffmpeg_kv = progress.split('\n').map(|kv| kv.split_once('='));

        let mut progress = FFmpegProgress::default();

        for kv in ffmpeg_kv {
            let Some((key, value)) = kv else { continue };

            let key = key.trim();
            let value = value.trim();

            match key {
                "frame" => progress.frame = value.parse::<usize>().ok(),
                "fps" => progress.fps = value.parse::<usize>().ok(),
                "bitrate" => progress.bitrate = value.split_once("kbits").map(|(v, _)| v.parse::<f32>().ok()).flatten(),
                "total_size" => progress.total_size = value.parse::<usize>().ok(),
                "out_time_us" => progress.out_time_us = value.parse::<usize>().ok(),
                "out_time_ms" => progress.out_time_ms = value.parse::<usize>().ok(),
                "dup_frames" => progress.dup_frames = value.parse::<usize>().ok(),
                "drop_frames" => progress.drop_frames = value.parse::<usize>().ok(),
                "speed" => progress.speed = value.split_once("x").map(|(v, _)| v.parse::<f32>().ok()).flatten(),
                "progress" => progress.progress = value.parse::<FFmpegProgressStatus>().ok(),
                _ => {  }
            }
        }

        progress
    }
}

pub struct FFmpegCommand {
    inner_child: Child,
}

impl FFmpegCommand {
    pub fn stop(mut self) -> std::io::Result<()> {
        self.inner_child.stdin
            .take().expect("Stdin has been taken")
            .write(b"q")?;
        
        self.inner_child.wait()?;
        self.force_stop()?;

        Ok(())
    }

    pub fn force_stop(mut self) -> std::io::Result<()> {
        self.inner_child.kill()?;

        Ok(())
    }
    
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.inner_child.wait()
    }

    /// Used for piping input or command to FFmpeg 
    pub fn stdin(&self) -> &Option<ChildStdin> {
        &self.inner_child.stdin
    }

    /// Used for piping output from FFmpeg 
    pub fn stdout(&self) -> &Option<ChildStdout> {
        &self.inner_child.stdout
    }

    /// Used for piping log from FFmpeg 
    pub fn stderr(&self) -> &Option<ChildStderr> {
        &self.inner_child.stderr
    }

    /// Used for piping input or command to FFmpeg 
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.inner_child.stdin.take()
    }

    /// Used for piping output from FFmpeg 
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.inner_child.stdout.take()
    }

    /// Used for piping log from FFmpeg 
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.inner_child.stderr.take()
    }
}

impl Drop for FFmpegCommand {
    fn drop(&mut self) {
        // Make sure that there is no zombie process
        let _ = self.inner_child.kill();
    }
}

pub trait Mode { }

pub struct Normal;
impl Mode for Normal { }

pub struct IO;
impl Mode for IO { }

pub struct FFmpegBuilder<M: Mode + ?Sized> {
    inner_command: Command,
    inner_args: Vec<String>,
    inserting_offset: Option<usize>,
    marker: PhantomData<M>
}

impl<A: Mode> FFmpegBuilder<A> {
    fn into<B: Mode>(self) -> FFmpegBuilder<B> {
        FFmpegBuilder { marker: PhantomData, inner_command: self.inner_command, inner_args: self.inner_args, inserting_offset: self.inserting_offset }
    }
}

impl FFmpegBuilder<Normal> {
    /// Start a new FFmpeg child process
    pub fn start(&mut self) -> anyhow::Result<FFmpegCommand> {
        self.inner_command.args(&self.inner_args);

        let inner_child = self.inner_command.spawn()?;

        Ok(FFmpegCommand { inner_child })
    }

    /// Start a new FFmpeg child process & listen to the progress
    pub fn start_listen_progress(mut self, progress_rx: &mut Option<Receiver<FFmpegProgress>>) -> anyhow::Result<FFmpegCommand> {
        let progress_pipe = Pipe::create_pipe()?;
        self.inner_args.extend(["-progress".to_owned(), progress_pipe.path().display().to_string()]);

        let (ffmpeg_progress_tx, ffmpeg_progress_rx) = channel(128);

        *progress_rx = Some(ffmpeg_progress_rx);

        std::thread::spawn(move || {
            let mut listener = progress_pipe.listen().unwrap();

            let mut has_ended = false;

            while !has_ended {
                let mut progress_string = String::new();

                let mut buffer = [0u8; 1024];
                let Ok(len) = listener.read(&mut buffer) else { continue };

                progress_string.push_str(&String::from_utf8_lossy(&buffer[..len]).trim());

                if progress_string.ends_with("end") { has_ended = true };

                let ffmpeg_progress = FFmpegProgress::from(progress_string);

                let ffmpeg_progress_tx = ffmpeg_progress_tx.clone();
                std::thread::spawn(move || ffmpeg_progress_tx.blocking_send(ffmpeg_progress).unwrap());
            }
        });

        self.start()
    }

    /// Inspect FFmpeg arguments
    pub fn inspect_args<F>(self, mut f: F) -> Self
    where
        Self: Sized,
        F: FnMut(&Vec<String>),
    {
        f(&self.inner_args);

        self
    }
    
    pub fn stdin(mut self, cfg: impl Into<Stdio>) -> Self {
        self.inner_command.stdin(cfg);

        self
    }

    pub fn stdout(mut self, cfg: impl Into<Stdio>) -> Self {
        self.inner_command.stdout(cfg);

        self
    }

    pub fn stderr(mut self, cfg: impl Into<Stdio>) -> Self {
        self.inner_command.stderr(cfg);

        self
    }

    pub fn input_with_pipe(mut self, pipe: &mut Option<Pipe>) -> anyhow::Result<FFmpegBuilder<IO>> {
        self.inserting_offset = Some(self.inner_args.len());
        
        *pipe = Some(Pipe::create_pipe()?);

        self.inner_args.extend(["-i".to_string(), pipe.as_ref().unwrap().path().display().to_string()]);

        Ok(self.into())
    }
    
    pub fn output_with_pipe(mut self, pipe: &mut Option<Pipe>) -> anyhow::Result<FFmpegBuilder<IO>> {
        self.inserting_offset = Some(self.inner_args.len());
        
        *pipe = Some(Pipe::create_pipe()?);

        self.inner_args.extend(["-y".to_string(), pipe.as_ref().unwrap().path().display().to_string()]);

        Ok(self.into())
    }

    pub fn input_with_file(mut self, path: PathBuf) -> FFmpegBuilder<IO> {
        self.inserting_offset = Some(self.inner_args.len());

        self.inner_args.extend(["-i".to_string(), path.display().to_string()]);

        self.into()
    }

    pub fn output_as_file(mut self, path: PathBuf) -> FFmpegBuilder<IO> {
        self.inserting_offset = Some(self.inner_args.len());

        self.inner_args.extend(["-y".to_string(), path.display().to_string()]);

        self.into()
    }

    pub fn input(mut self, buffer: &[u8]) -> std::io::Result<FFmpegBuilder<IO>> {
        let path = random_temp_file();

        let mut file = OpenOptions::new().read(true).write(true).create_new(true).open(&path)?;
        file.write(buffer)?;

        self.inserting_offset = Some(self.inner_args.len());

        self.inner_args.extend(["-i".to_string(), path.display().to_string()]);

        Ok(self.into())
    }

    pub fn output(mut self, file: &mut Option<File>) -> std::io::Result<FFmpegBuilder<IO>> {
        let path = random_temp_file();

        *file = Some(OpenOptions::new().read(true).write(true).create_new(true).open(&path)?);

        self.inserting_offset = Some(self.inner_args.len());
        
        self.inner_args.extend(["-y".to_string(), path.display().to_string()]);

        Ok(self.into())
    }

    /// Add custom argument to FFmpeg process
    ///
    /// Must take into consideration of where this argument is located
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.inner_args.push(arg.as_ref().to_string_lossy().to_string());
        
        self
    }

    /// Add custom argument to FFmpeg process
    ///
    /// Must take into consideration of where this argument is located
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self = self.arg(arg);
        }

        self
    }
}

impl FFmpegBuilder<IO> {
    /// Set format
    pub fn format(mut self, format: impl AsRef<str>) -> Self {
        let at = self.inserting_offset.unwrap_or(self.inner_args.len());
        self.inner_args.splice(at..at, ["-f".to_string(), format.as_ref().to_string()]);

        self
    }

    /// Set audio codec
    pub fn codec_audio(self, codec: impl AsRef<str>) -> Self {
        self.args(["-c:a", codec.as_ref()])
    }

    /// Set video codec
    pub fn codec_video(self, codec: impl AsRef<str>) -> Self {
        self.args(["-c:v", codec.as_ref()])
    }

    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.inner_args.insert(self.inserting_offset.unwrap_or(self.inner_args.len()), arg.as_ref().to_string_lossy().to_string());

        self.inserting_offset.as_mut().map(|v| v.add_assign(1));
        
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self = self.arg(arg);
        }

        self
    }

    pub fn done(mut self) -> FFmpegBuilder<Normal> {
        self.inserting_offset = None;
        self.into()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FFmpegDownloadProgress {
    Starting,
    /// An option because the content-length might not be available
    Downloading(Option<usize>),
    Extracting,
    Finished
}

pub struct FFmpeg;

impl FFmpeg {
    /// Uses [`FFmpeg::get_program`] to find the FFmpeg program
    ///
    /// Panic if doesn't exist
    pub fn new() -> FFmpegBuilder<Normal> {
        let program = Self::get_program().expect("Failed to find FFmpeg").expect("Can't find FFmpeg in your system");
        
        Self::new_with_program(program)
    }

    /// Must provide a valid FFmpeg program path
    pub fn new_with_program<S: AsRef<OsStr>>(program: S) -> FFmpegBuilder<Normal> {
        let mut inner_command = Command::new(program);

        inner_command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        
        FFmpegBuilder {
            inner_command,
            inner_args: vec![].into(),
            inserting_offset: Some(0),
            marker: PhantomData
        }
    }

    /// Override the download FFmpeg directory
    ///
    /// # Safety
    /// This should be called before any other function is called, or there will be inconsistencies of the downloaded FFmpeg directory
    pub fn override_downloaded_ffmpeg_path(path: PathBuf) -> anyhow::Result<()> {
        // SAFETY: override the FFmpeg directory, this SHOULD be called before any of the stuff is called
        unsafe { *FFMPEG_DOWNLOAD_ROOT_DIR = path  };
        Ok(())
    }

    /// Check if FFmpeg is exist in the current environment
    pub fn is_exist_in_env() -> bool {
        match Command::new("ffmpeg").spawn() {
            Ok(mut child) => {
                let _ = child.kill();
    
                true
            },
            Err(err) => match err.kind() {
                _ => false
            }
        }
    }

    /// Downloaded FFmpeg folder
    pub fn downloaded_ffmpeg_folder() -> anyhow::Result<PathBuf> {
        // SAFETY: APP_DIR defaults to current_exe, which SHOULD be overidden before any of the stuff is called
        unsafe { Ok(FFMPEG_DOWNLOAD_ROOT_DIR.join("ffmpeg")) }
    }

    /// Downloaded FFmpeg executable
    pub fn downloaded_ffmpeg_path() -> anyhow::Result<PathBuf> {
        Ok(Self::downloaded_ffmpeg_folder()?.join("ffmpeg"))
    }

    /// Check if FFmpeg is already downloaded
    ///
    /// Doesn't mean that it exist in the current environmant
    pub fn is_downloaded() -> anyhow::Result<bool> {
        match Self::downloaded_ffmpeg_path() {
            Ok(path) => Ok(path.exists()),
            Err(err) => Err(err),
        }
    }
    
    /// Get the program string that can be used for [`Command::new`]
    pub fn get_program() -> anyhow::Result<Option<String>> {
        if Self::is_exist_in_env() { return Ok(Some("ffmpeg".to_string())) };
        if !Self::is_downloaded()? { return Ok(None) };
    
        match Self::downloaded_ffmpeg_path() {
            Ok(path) => Ok(Some(path.display().to_string())),
            Err(err) => Err(err),
        }
    }

    /// Returns the read channel for listening the download state & the thread handle
    ///
    /// Returns [`Option::None`] if FFmpeg alredy exist
    ///
    /// It is your responsibility for making sure that the download is succeed & finished!
    pub fn auto_download() -> impl std::future::Future<Output = anyhow::Result<Option<(JoinHandle<Result<(), anyhow::Error>>, Receiver<FFmpegDownloadProgress>)>>> {
        FFmpeg::auto_download_with_url(FFMPEG_URL)
    }

    /// Downloaded file must be compressed in GZIP archive that contains the single FFmpeg binary
    ///
    /// Consider looking at this
    /// https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.0
    ///
    /// Returns the read channel for listening the download state & the thread handle
    ///
    /// Returns [`Option::None`] if FFmpeg alredy exist
    ///
    /// It is your responsibility for making sure that the download is succeed & finished!
    pub async fn auto_download_with_url(url: &str) -> anyhow::Result<Option<(JoinHandle<Result<(), anyhow::Error>>, Receiver<FFmpegDownloadProgress>)>> {
        if Self::get_program()?.is_some() { return Ok(None) };

        let mut response = reqwest::get(url).await?;
        let length = response.content_length();

        let (progress_tx, progress_rx): (Sender<FFmpegDownloadProgress>, _) = channel(256);

        let handle = tokio::task::spawn(async move {
            let mut buffer = Vec::new();

            // SAFETY: we just don't care, this doesn't matter really
            let _ = progress_tx.send(FFmpegDownloadProgress::Starting).await;

            let mut downloaded = 0;
            while let Some(chunk) = response.chunk().await? {
                downloaded += chunk.len();
                buffer.extend(chunk);

                let length = match length {
                    Some(length) => Some(((downloaded as f32 / length as f32) * 100.0) as usize),
                    None => None,
                };

                // SAFETY: we just don't care, this doesn't matter really
                let _ = progress_tx.send(FFmpegDownloadProgress::Downloading(length)).await;
            }

            // SAFETY: we just don't care, this doesn't matter really
            let _ = progress_tx.send(FFmpegDownloadProgress::Extracting).await;

            let mut gz = GzDecoder::new(Cursor::new(buffer));

            let mut binary = Vec::new();
            gz.read_to_end(&mut binary)?;

            let output_path = Self::downloaded_ffmpeg_folder()?;
            std::fs::create_dir_all(&output_path)?;

            let ffmpeg_path = output_path.join("ffmpeg");
            std::fs::write(&ffmpeg_path, binary)?;

            #[cfg(all(target_family = "unix"))]
            {
                use std::os::unix::fs::PermissionsExt;
                
                std::fs::set_permissions(ffmpeg_path, std::fs::Permissions::from_mode(0o755))?;
            }

            Self::get_program()?.context("Failed to download FFmpeg")?;

            // SAFETY: we just don't care, this doesn't matter really
            let _ = progress_tx.send(FFmpegDownloadProgress::Finished).await;

            Ok::<(), anyhow::Error>(())
        });

        Ok(Some((handle, progress_rx)))
    }
}

pub(crate) fn random_string() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10)
        .map(char::from)
        .collect()
}

pub(crate) fn random_temp_file() -> PathBuf {
    let name: String = random_string();

    temp_dir().join(name)
}
