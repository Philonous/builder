use std::{
    fs::create_dir_all,
    path::{absolute, PathBuf},
};

pub struct TempDir(PathBuf);

impl std::ops::Deref for TempDir {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        tracing::debug!("Dropping temp path: {:?}", self.0);
        if let None = std::env::var_os("KEEP_TEST_DIRS") {
            std::fs::remove_dir_all(&self.0).expect(&format!("deleting temp dir {:?}", &self.0));
        }
    }
}

impl TempDir {
    pub fn create(path: &str) -> Self {
        create_dir_all("tmp").expect("create tmp dir");
        let path = absolute("tmp").expect("absolute tmp").join(path);
        tracing::debug!("Creating temp path: {:?}", path);
        std::fs::remove_dir_all(&path).expect(&format!("deleting temp dir {:?}", &path));
        std::fs::create_dir(&path).expect(&format!("creating temp dir {:?}", path));
        TempDir(path)
    }
}
