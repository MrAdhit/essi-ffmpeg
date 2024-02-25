use std::{fs, io::Read, process::Stdio, time::Duration};

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
    
    let mut output_mp4 = None;

    // https://trac.ffmpeg.org/wiki/Capture/Desktop#UseWindows8DesktopDuplicationAPI
    let mut ffmpeg = essi_ffmpeg::FFmpeg::new()
        .stderr(Stdio::inherit())
        .args(["-init_hw_device", "d3d11va"])
        .args(["-filter_complex", "ddagrab=0"])
        .output(&mut output_mp4).unwrap()
            .codec_video("h264_nvenc")
            .args(["-cq:v", "20"])
            .format("mp4")
            .done()
        .inspect_args(|args| {
            dbg!(args);
        });

    let ffmpeg = ffmpeg.start().unwrap();

    // Record for 5 seconds
    std::thread::sleep(Duration::from_secs(5));

    ffmpeg.stop().unwrap();

    let mut output_mp4_buffer = Vec::new();

    output_mp4.expect("No stream is written").read_to_end(&mut output_mp4_buffer).unwrap();

    fs::write("screen_output.mp4", output_mp4_buffer).unwrap();

    Ok(())
}