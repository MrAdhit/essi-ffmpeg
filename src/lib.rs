#![feature(type_changing_struct_update)]

use std::{env::{current_exe, temp_dir}, ffi::OsStr, fs::File, io::{Cursor, Error, Read, Write}, marker::PhantomData, ops::AddAssign, path::PathBuf, process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio}};

use flate2::read::GzDecoder;
use rand::{distributions::Alphanumeric, Rng};
use tokio::{sync::mpsc::{channel, Receiver, Sender}, task::JoinHandle};

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

pub struct FFmpegCommand {
    inner_child: Child
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
        FFmpegBuilder { marker: PhantomData, ..self }
    }
}

impl FFmpegBuilder<Normal> {
    /// Start a new FFmpeg child process
    pub fn start(&mut self) -> Result<FFmpegCommand, Error> {
        self.inner_command.args(&self.inner_args);

        let inner_child = self.inner_command.spawn()?;

        Ok(FFmpegCommand { inner_child })
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

        let mut file = File::create_new(&path)?;
        file.write(buffer)?;

        self.inserting_offset = Some(self.inner_args.len());

        self.inner_args.extend(["-i".to_string(), path.display().to_string()]);

        Ok(self.into())
    }

    pub fn output(mut self, file: &mut Option<File>) -> std::io::Result<FFmpegBuilder<IO>> {
        let path = random_temp_file();

        *file = Some(File::create_new(&path)?);

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
        Ok(
            current_exe()?
                .parent().expect("Can't get the current program folder.\nThis should never fail... I think")
                .join("ffmpeg")
        )
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
            std::fs::write(output_path.join("ffmpeg"), binary)?;

            // SAFETY: we just don't care, this doesn't matter really
            let _ = progress_tx.send(FFmpegDownloadProgress::Finished).await;

            Ok::<(), anyhow::Error>(())
        });

        Ok(Some((handle, progress_rx)))
    }
}

fn random_temp_file() -> PathBuf {
    let name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10)
        .map(char::from)
        .collect();

    temp_dir().join(name)
}