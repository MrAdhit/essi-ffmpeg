use std::process::Stdio;

use essi_ffmpeg::FFmpeg;

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

    // Build and execute an FFmpeg command
    let mut ffmpeg = FFmpeg::new()
        .stderr(Stdio::inherit())
        .input_with_file("examples/assets/sample.flv".into()).done()
        .output_as_file("output_file.mp4".into()).done()
        .start().unwrap();

    ffmpeg.wait().unwrap();

    Ok(())
}
