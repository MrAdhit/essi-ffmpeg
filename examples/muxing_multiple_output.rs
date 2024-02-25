use std::{fs, io::Read, process::Stdio};

use essi_ffmpeg::FFmpeg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Some((handle, mut progress)) = FFmpeg::auto_download().await.unwrap() {
        tokio::spawn(async move {
            while let Some(state) = progress.recv().await {
                match state {
                    essi_ffmpeg::FFmpegDownloadProgress::Starting => println!("Starting to download FFmpeg"),
                    essi_ffmpeg::FFmpegDownloadProgress::Downloading(progress) => println!("Downloading FFmpeg{}", progress.map(|p| format!(": {p} %")).unwrap_or_default()),
                    essi_ffmpeg::FFmpegDownloadProgress::Extracting => println!("Extracting FFmpeg"),
                    essi_ffmpeg::FFmpegDownloadProgress::Finished => println!("Finished downloading FFmpeg"),
                }
            }
        });

        handle.await.unwrap().unwrap();
    } else {
        println!("FFmpeg is downloaded, using existing installation");
    }

    let buffer = include_bytes!("./assets/sample.flv");
    
    let mut output_mp4 = None;
    let mut output_mov = None;

    let mut ffmpeg = essi_ffmpeg::FFmpeg::new()
        .stderr(Stdio::inherit())
        .input(buffer).unwrap()
            .format("flv")
            .done()
        .output(&mut output_mp4).unwrap()
            .codec_audio("copy")
            .format("mp4")
            .done()
        .output(&mut output_mov).unwrap()
            .codec_audio("copy")
            .format("mov")
            .done()
        .output_as_file("output.webm".into())
            .codec_audio("libvorbis")
            .codec_video("libvpx")
            .format("webm")
            .done()
        .inspect_args(|arg| {
            dbg!(arg);
        });

    let mut ffmpeg = ffmpeg.start().unwrap();

    ffmpeg.wait().unwrap();

    let mut output_mp4_buffer = Vec::new();
    let mut output_mov_buffer = Vec::new();

    output_mp4.expect("No stream is written").read_to_end(&mut output_mp4_buffer).unwrap();
    output_mov.expect("No stream is written").read_to_end(&mut output_mov_buffer).unwrap();

    fs::write("output.mp4", output_mp4_buffer).unwrap();
    fs::write("output.mov", output_mov_buffer).unwrap();

    Ok(())
}