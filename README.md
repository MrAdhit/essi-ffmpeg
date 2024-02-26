# essi-ffmpeg

Use FFmpeg easily, includes downloading FFmpeg binaries, executing FFmpeg commands, and handling I/O processing tasks. Provides a simplified interface for utilizing the power of FFmpeg.

## Features

- **Automatic FFmpeg Download**: Automatically download FFmpeg binaries suitable for your platform if FFmpeg is not found in the environment.
- **Flexible Command Execution**: Build and execute FFmpeg commands with ease for handling various multimedia processing tasks.

## Getting Started

### Adding to Your Project

Add `essi-ffmpeg` to your `Cargo.toml` dependencies:

```toml
[dependencies]
essi-ffmpeg = { git = "https://github.com/MrAdhit/essi-ffmpeg" }
```

### Basic Usage

```rust
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
        .stderr(std::process::Stdio::inherit())
        .input_with_file("input_file.flv".into()).done()
        .output_as_file("output_file.mp4".into()).done()
        .start().unwrap();

    ffmpeg.wait().unwrap();

    Ok(())
}
```

## Examples

This crate includes several examples demonstrating different use cases:

- `basic_usage.rs`: Shows the basic usage of using this library.
- `muxing_hardware_accelerated.rs`: Demonstrates muxing with hardware acceleration.
- `muxing_multiple_output.rs`: Shows how to mux multiple outputs.
- `override_ffmpeg_download_directory.rs`: Illustrates how to override the default FFmpeg download directory.
- `windows_screen_recorder.rs`: Provides an example of a simple screen recorder on Windows using FFmpeg.

To run an example, use:

```shell
cargo run --example example_name
```

##  Contributing

Contributions are welcome! Here are several ways you can contribute:

- **[Report Issues](https://github.com/MrAdhit/essi-ffmpeg/issues)**: Submit bugs found or log feature requests for the `essi-ffmpeg` project.
- **[Submit Pull Requests](https://github.com/MrAdhit/essi-ffmpeg/pulls)**: Review open PRs, and submit your own PRs.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
