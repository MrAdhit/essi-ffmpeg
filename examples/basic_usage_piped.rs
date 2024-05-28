use std::{fs::File, io::{ErrorKind, Read, Write}, process::Stdio, sync::{atomic::{AtomicBool, Ordering}, Arc}};

use essi_ffmpeg::{pipe::Piped, FFmpeg};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Automatically download FFmpeg if not found
    if let Some((handle, mut progress)) = FFmpeg::auto_download().await.unwrap() {
        tokio::spawn(async move {
            while let Some(state) = progress.recv().await {
                println!("{:?}", state);
            }
        });

        handle.await.unwrap().unwrap();
    } else {
        println!("FFmpeg is downloaded, using existing installation");
    }

    let mut input = None;
    let mut output = None;
    let mut ffmpeg_progress = None;

    // Build and execute an FFmpeg command
    let mut ffmpeg = FFmpeg::new()
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .args(["-hwaccel", "auto"])
        .input_with_pipe(&mut input).unwrap()
            .format("flv")
            .done()
        .output_with_pipe(&mut output).unwrap()
            .format("webm")
            .done()
        .inspect_args(|arg| println!("FFmpeg arguments: {arg:?}"))
        .start_listen_progress(&mut ffmpeg_progress).unwrap();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let input = input.unwrap();
            let mut input_listener = input.listen().unwrap();

            let mut input_video = File::open("examples/assets/sample.flv").unwrap();

            loop {
                let mut buffer = [0u8; 64];

                let len = match input_video.read(&mut buffer) {
                    Ok(len) => len,
                    Err(ref err) if err.kind() == ErrorKind::WouldBlock => { continue },
                    Err(err) => panic!("{err}"),
                };

                input_listener.write(&buffer).unwrap();

                if len < buffer.len() { break };
            }
        });

        let is_encoding = Arc::new(AtomicBool::new(true));

        scope.spawn({
            let is_encoding = is_encoding.clone();

            move || {
                loop {
                    let progress = ffmpeg_progress.as_mut().expect("FFmpeg is not started yet").blocking_recv().unwrap();
                    match progress.progress {
                        Some(progress) => if let essi_ffmpeg::FFmpegProgressStatus::End = progress {
                            break;
                        },
                        _ => { }
                    }
                }
    
                is_encoding.store(false, Ordering::Relaxed);
            }
        });

        scope.spawn(move || {
            let output = output.unwrap();
            let mut output_listener = output.listen().unwrap();

            let mut output_video = File::create("output.webm").unwrap();

            while is_encoding.load(Ordering::Acquire) {
                let mut buffer = [0u8; 64];

                match output_listener.read(&mut buffer) {
                    Ok(len) => len,
                    Err(ref err) if err.kind() == ErrorKind::WouldBlock => { continue },
                    Err(err) => panic!("{err}"),
                };

                output_video.write(&buffer).unwrap();
            }
        });
    });

    ffmpeg.wait().unwrap();

    Ok(())
}
