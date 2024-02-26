use std::env::current_dir;

use essi_ffmpeg::FFmpeg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Override to current working directory
    let _ = FFmpeg::override_downloaded_ffmpeg_path(current_dir().unwrap());

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

    Ok(())
}