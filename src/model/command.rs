pub struct Command {
    pub executable: String,
    pub args: Vec<String>,
}

impl<'a> std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.executable)?;
        for arg in self.args.iter() {
            write!(f, " {}", arg)?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("command finshed with error")]
    ExitFailure,
    #[error("Error while executing command: {0}")]
    IOError(std::io::Error),
}

impl<'a> Command {
    pub fn exe(&self) -> Result<(), Error> {
        let mut proc = std::process::Command::new(&*self.executable);
        // Add arguments
        for arg in &self.args {
            proc.arg(&**arg);
        }

        eprintln!("build: {}", self);
        // Execute command
        match proc.status() {
            Ok(status) => {
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::ExitFailure)
                }
            }
            Err(e) => Err(Error::IOError(e)),
        }
    }
}
