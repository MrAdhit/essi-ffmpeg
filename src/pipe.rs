use std::{any::Any, io, path::{Path, PathBuf}};

#[cfg(windows)]
mod windows;

#[cfg(unix)]
use nix::unistd;

pub trait Piped
where
    Self: Sized,
{
    fn create_pipe() -> anyhow::Result<Self> {
        Self::create_pipe_with_name(super::random_string())
    }

    fn create_pipe_with_name(name: String) -> anyhow::Result<Self>;
    fn create_pipe_with_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self>;
    fn connect_pipe_with_name(name: String) -> anyhow::Result<impl io::Read + io::Write>;
    fn connect_pipe_with_path<P: AsRef<Path>>(path: P) -> anyhow::Result<impl io::Read + io::Write>;
    fn listen(self) -> anyhow::Result<impl io::Read + io::Write>;
}

#[allow(dead_code)]
pub struct Pipe
{
    path: PathBuf,
    pipe: Box<dyn Any + Send>,
}

impl Pipe {
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

#[cfg(windows)]
impl Piped for Pipe {
    fn create_pipe_with_name(name: String) -> anyhow::Result<Self> {
        Self::create_pipe_with_path(format!("//./pipe/{}", name))
    }

    fn create_pipe_with_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let pipe = windows::PipeListener::bind(&path)?;

        Ok(Self {
            path: path.as_ref().into(),
            pipe: Box::new(pipe),
        })
    }

    fn connect_pipe_with_name(name: String) -> anyhow::Result<impl std::io::Read + std::io::Write> {
        Self::connect_pipe_with_path(format!("//./pipe/{}", name))
    }

    fn connect_pipe_with_path<P: AsRef<Path>>(path: P) -> anyhow::Result<impl std::io::Read + std::io::Write> {
        Ok(windows::PipeStream::connect(path)?)
    }
    
    fn listen(self) -> anyhow::Result<impl std::io::Read + std::io::Write> {
        use windows::PipedListener;
        use std::ops::DerefMut;

        let mut binding = match self.pipe.downcast::<windows::PipeListener>() {
            Ok(pipe) => pipe,
            Err(_) => anyhow::bail!("Error when downcasting the pipe"),
        };
        let pipe = binding.deref_mut();

        Ok(pipe.accept()?)
    }
}

#[cfg(unix)]
impl Piped for Pipe {
    fn create_pipe_with_name(name: String) -> anyhow::Result<Self> {
        Self::create_pipe_with_path(std::env::temp_dir().join(format!("{name}.pipe")))
    }

    /// Will try to delete the file in path if it exists
    fn create_pipe_with_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        use nix::sys::stat::Mode;

        let _ = std::fs::remove_file(path.as_ref());
        unistd::mkfifo(path.as_ref(), Mode::all())?;
        
        Ok(Pipe {
            path: path.as_ref().into(),
            pipe: Box::new(())
        })
    }

    fn connect_pipe_with_name(name: String) -> anyhow::Result<impl std::io::Read + std::io::Write> {
        Self::connect_pipe_with_path(std::env::temp_dir().join(format!("{name}.pipe")))
    }

    fn connect_pipe_with_path<P: AsRef<Path>>(path: P) -> anyhow::Result<impl std::io::Read + std::io::Write> {
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .open(path)?;
        
        Ok(file)
    }

    fn listen(self) -> anyhow::Result<impl std::io::Read + std::io::Write> {
        Self::connect_pipe_with_path(self.path.clone())
    }
}

#[cfg(test)]
mod test {
    use io::{Read, Write};

    use super::*;

    #[test]
    fn piping() -> anyhow::Result<()> {
        let pipe_name = "pipe".to_owned();

        let static_test_data = "bello".to_owned();
        let random_test_data = crate::random_string();

        let listener_pipe = Pipe::create_pipe_with_name(pipe_name.clone())?;

        let task = std::thread::spawn({
            let static_test_data = static_test_data.clone();
            let random_test_data = random_test_data.clone();
            
            move || {
                let mut reader = listener_pipe.listen()?;

                let mut buffer_test_data = [0u8; 5];
                reader.read_exact(&mut buffer_test_data)?;
                assert_eq!(static_test_data.as_bytes(), &buffer_test_data);
    
                let mut buffer_random_test_data = Vec::new();

                loop {
                    let mut buffer = [0u8; 64];

                    let len = reader.read(&mut buffer)?;

                    buffer_random_test_data.extend(&buffer[..len]);

                    if len < buffer.len() { break }
                }

                assert_eq!(random_test_data.as_bytes(), &buffer_random_test_data);
    
                anyhow::Ok(())
            }
        });

        let mut writer = Pipe::connect_pipe_with_name(pipe_name.clone())?;

        writer.write(static_test_data.as_bytes())?;
        writer.write(random_test_data.as_bytes())?;

        task.join().unwrap()?;

        Ok(())
    }
}
