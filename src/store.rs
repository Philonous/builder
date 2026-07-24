use parking_lot::Mutex;
use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{ErrorKind, Read, Seek, Write},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, std::io::Error>;

fn append_record_raw(file: &mut File, record: &[u8]) -> Result<()> {
    file.write_all(&(record.len() as u32).to_be_bytes())?;
    file.write_all(record)?;
    Ok(())
}

/// Stores records (list of arbitrary slices of u8) in a file - append
/// only. Recognizes incomplete writes and truncates file on read.
/// No CRC or similar for now
///
/// Usage note: This code assumes that the consumer first does a full
/// read, then starts calling append. append before reads reached the
/// end will panic, reads after the end has been reached once will always
/// return None, even if more records have been added in the meantime.
pub struct Store {
    // Absolute path
    pub path: PathBuf,
    file: Mutex<File>,
    // Length of file at start (and never updated) so we don't have to
    // read metadata on every read. Becomes irrelevant once we have
    // ingested the entire file
    length: u64,
    // Reached end once via read_record, switched to write mode
    reached_end: bool,
}

impl Store {
    /// Open database, returning Some(store) on success, None when the
    /// databse is locked. Blocks until the database is available when
    /// block is true
    pub fn open(path: impl AsRef<Path>, block: bool) -> Result<Option<Self>> {
        let path: PathBuf = std::path::absolute(path)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;

        match file.try_lock() {
            Ok(_) => {} // File now locked
            Err(TryLockError::WouldBlock) => {
                tracing::info!(
                    "Store file {:?} locked, waiting for other processes to finish",
                    path
                );
                if block {
                    file.lock()?
                } else {
                    return Ok(None);
                };
            }
            Err(TryLockError::Error(e)) => return Err(e),
        }

        // Sync directory
        File::open(path.parent().expect("parent directory of absolute path"))?
            .sync_all()?;

        let length = file.metadata()?.len();
        Ok(Some(Store {
            path,
            file: Mutex::new(file),
            length,
            reached_end: false,
        }))
    }

    // Force-create a new database
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path: PathBuf = std::path::absolute(path)?;
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        match file.try_lock() {
            Ok(_) => {}
            Err(TryLockError::Error(e)) => {
                return Err(e);
            }
            Err(TryLockError::WouldBlock) => {
                tracing::error!(
                    "EWOULDBLOCK while locking newly created file {:?}",
                    &path
                );
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
        }
        // Sync directory after creating file
        File::open(path.parent().expect("parent directory of absolute path"))?
            .sync_all()?;

        Ok(Store {
            path,
            file: Mutex::new(file),
            length: 0,
            reached_end: true,
        })
    }

    pub fn next_record(&mut self) -> Result<Option<Vec<u8>>> {
        if self.reached_end {
            return Ok(None);
        }
        let mut file = self.file.lock();
        // Position *before* we read, so we know here to truncate when
        // we see an incomplete record
        let pos = file.stream_position()?;
        let mut lenbuf = [0u8; 4];

        match file.read_exact(&mut lenbuf) {
            Ok(()) => {}
            // EOF (normal) or truncated length field
            // TODO: perhaps check which it is? Not that it matters
            // overly much
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                // This is a noop in the EOF case and fixes the
                // database in the truncated length field case
                // No warning here because this is the expected case
                file.set_len(pos)?;
                // Also set stream position
                file.seek(std::io::SeekFrom::Start(pos))?;
                self.reached_end = true;
                return Ok(None);
            }
            Err(e) => Err(e)?,
        }

        let len = u32::from_be_bytes(lenbuf);
        // position + 4 length bytes + length of record
        //
        // We don't expect record > 1MB, treat as corrupted and
        // truncate This also protects against malicious files that
        // try to trick us into mallocking huge amounts of memory
        if pos + 4 + len as u64 > self.length || len > 1024 * 1024 {
            // Ragged record found, truncate file to chop it off
            tracing::warn!("DB: truncated record found");
            file.set_len(pos)?;
            // Also set stream position
            file.seek(std::io::SeekFrom::Start(pos))?;
            self.reached_end = true;
            return Ok(None);
        }

        let mut data = vec![0; len as usize];

        // We have already checked that the length is sufficient, if
        // we hit EOF anyway something went wrong and we return an
        // error instead of handling it as a ragged record
        file.read_exact(&mut data)?;
        Ok(Some(data))
    }

    /// Append a record. This function panics unless the database has
    /// been read to the end (next_record() until None)
    pub fn append_record(&mut self, record: &[u8]) -> Result<()> {
        assert!(
            self.reached_end,
            "Store needs to be at end position before writing"
        );
        let mut file = self.file.lock();
        append_record_raw(&mut *file, record)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }
    /// Append multiple record. This function panics unless the database has
    /// been read to the end (next_record() until None)
    pub fn append_records<I>(&mut self, records: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        assert!(
            self.reached_end,
            "Store needs to be at end position before writing"
        );
        let mut file = self.file.lock();
        for record in records {
            append_record_raw(&mut *file, record.as_ref())?;
        }
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }

    pub fn mv(&mut self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::rename(&self.path, path.as_ref())?;
        // Sync target directory
        File::open(
            path.as_ref()
                .parent()
                .expect("parent directory of absolute path"),
        )?
        .sync_all()?;

        // Sync source directory
        File::open(
            self.path
                .parent()
                .expect("parent directory of absolute path"),
        )?
        .sync_all()?;

        self.path = path.as_ref().to_owned();
        Ok(())
    }
}

impl Iterator for Store {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_record().transpose()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test::TempDir;

    static EXAMPLE: &[&[u8]] = &[&[1, 2, 3], &[], &[99]];

    #[test]
    fn simple_roundtrip() {
        let tmp = TempDir::create("test-simple-roundrip");
        let mut db = Store::open(tmp.join("db"), true).unwrap().unwrap();
        // We have to read to the end first
        assert_eq!(db.next_record().unwrap(), None);
        for record in EXAMPLE {
            db.append_record(record).unwrap();
        }
        drop(db);

        let db = Store::open(tmp.join("db"), true).unwrap().unwrap();
        let res = db.collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(res, EXAMPLE);
    }

    #[test]
    fn iterate_then_append() {
        let tmp = TempDir::create("iterate-then-append");
        let mut db = Store::open(tmp.join("db"), true).unwrap().unwrap();
        // We have to read to the end first
        for _ in &mut db {}
        for record in EXAMPLE {
            db.append_record(record).unwrap();
        }
        drop(db);

        let db = Store::open(tmp.join("db"), true).unwrap().unwrap();
        let res = db.collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(res, EXAMPLE);
    }

    #[test]
    fn ragged_record() {
        let tmp = TempDir::create("test-ragged-record");
        let mut file = File::create(tmp.join("db")).unwrap();
        // First record is complete, second record advertises 5 bytes
        // but contains only 2
        file.write_all(&[0, 0, 0, 3, 10, 11, 12, 0, 0, 0, 5, 1, 2])
            .unwrap();
        file.flush().expect("flush");
        file.sync_all().expect("sync_all");
        drop(file);
        let mut db = Store::open(tmp.join("db"), true).unwrap().unwrap();
        assert_eq!(
            db.next_record().unwrap().as_deref(),
            Some(&[10u8, 11, 12] as &[u8])
        );
        assert_eq!(db.next_record().unwrap().as_deref(), None);
        drop(db);
        // Check that the file is now cleaned
        assert_eq!(
            std::fs::read(tmp.join("db")).unwrap(),
            &[0, 0, 0, 3, 10, 11, 12]
        );
    }

    #[test]
    fn file_locking() {
        let tmp = TempDir::create("test-file-locking");
        let db = Store::open(tmp.join("db"), false).unwrap();
        assert!(matches!(db, Some(_)));
        drop(db);
        let db = Store::open(tmp.join("db"), true).unwrap();
        assert!(matches!(db, Some(_)));
        let db = Store::open(tmp.join("db"), false).unwrap();
        assert!(matches!(db, None));
    }
}
