use std::fmt;

use camino::Utf8PathBuf;
use jiff::Timestamp;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_rows;
use tracing::info;

use crate::Result;
use crate::ffprobe::FfProbe;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscodeStatus {
    Pending,
    Success,
    Error,
}

impl fmt::Display for TranscodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscodeStatus::Pending => write!(f, "Pending"),
            TranscodeStatus::Success => write!(f, "Success"),
            TranscodeStatus::Error => write!(f, "Error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodeFile {
    pub rowid: i64,
    pub path: Utf8PathBuf,
    pub status: TranscodeStatus,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub created_on: Timestamp,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub updated_on: Timestamp,
    pub error_message: Option<String>,
    pub file_size: i64,
    pub ffprobe_info: String,
}

impl TranscodeFile {
    pub fn ffprobe(&self) -> Option<FfProbe> {
        serde_json::from_str(&self.ffprobe_info).ok()
    }
}

#[derive(Debug)]
pub struct NewTranscodeFile {
    pub path: Utf8PathBuf,
    pub file_size: u64,
    pub ffprobe_info: FfProbe,
}

#[derive(Clone)]
pub struct Database {
    db: Pool<SqliteConnectionManager>,
}

impl Database {
    pub fn new() -> Result<Self> {
        let manager = SqliteConnectionManager::file("transcoder.db");
        let this = Self {
            db: Pool::new(manager)?,
        };
        this.init_database()?;
        Ok(this)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory();
        let this = Self {
            db: Pool::new(manager)?,
        };
        this.init_database()?;
        Ok(this)
    }

    fn init_database(&self) -> Result<()> {
        let sql = include_str!("../init_db.sql");
        let connection = self.db.get()?;
        connection.execute(sql, ())?;
        Ok(())
    }

    #[cfg(test)]
    pub fn insert(&self, file: NewTranscodeFile) -> Result<()> {
        let connection = self.db.get()?;
        let now = Timestamp::now().as_second();

        connection.execute("INSERT INTO transcode_files (path, created_on, updated_on, file_size) VALUES (?1, ?2, ?3, ?4)", params![
            file.path.as_str(),
            now,
            now,
            file.file_size as i64,
        ])?;

        Ok(())
    }

    pub fn list(&self) -> Result<Vec<TranscodeFile>> {
        self.list_limit(None)
    }

    pub fn list_limit(&self, count: Option<i64>) -> Result<Vec<TranscodeFile>> {
        let connection = self.db.get()?;
        let mut statement = connection
            .prepare("SELECT rowid, * FROM transcode_files ORDER BY file_size DESC LIMIT ?1")?;
        let res = from_rows::<TranscodeFile>(statement.query([count.unwrap_or(i64::MAX)])?);
        let rows: Result<_, serde_rusqlite::Error> = res.collect();
        Ok(rows?)
    }

    pub fn insert_batch(&self, files: &[NewTranscodeFile]) -> Result<()> {
        info!("inserting batch of {} files", files.len());
        let mut connection = self.db.get()?;

        let now = Timestamp::now().as_second();
        let tx = connection.transaction()?;
        {
            let mut statement = tx.prepare("INSERT INTO transcode_files (path, created_on, updated_on, file_size, ffprobe_info) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (path) DO NOTHING")?;
            for file in files {
                let json_info = serde_json::to_string(&file.ffprobe_info)?;
                statement.execute(params![
                    file.path.as_str(),
                    now,
                    now,
                    file.file_size as i64,
                    json_info
                ])?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    pub fn set_file_status(
        &self,
        rowid: i64,
        status: TranscodeStatus,
        error_message: Option<String>,
    ) -> Result<()> {
        info!("Setting file status for rowid {} to {:?}", rowid, status);
        let connection = self.db.get()?;
        let now = Timestamp::now().as_second();
        connection.execute(
            "UPDATE transcode_files SET status = ?1, updated_on = ?2, error_message = ?3 WHERE rowid = ?4",
            params![status as i32, now, error_message, rowid],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffprobe::ffprobe;

    #[test]
    fn test_insert_row() -> Result<()> {
        let db = Database::in_memory()?;

        db.insert(NewTranscodeFile {
            path: "/stuff/1.mp4".into(),
            file_size: 696969,
            ffprobe_info: FfProbe::default(),
        })?;

        let rows = db.list()?;
        assert_eq!(1, rows.len());
        assert_eq!(696969, rows[0].file_size);

        Ok(())
    }

    #[test]
    fn test_insert_batch() -> Result<()> {
        let db = Database::in_memory()?;

        let files: Vec<_> = (0..100)
            .map(|i| NewTranscodeFile {
                path: format!("/stuff/{i}.mp4").into(),
                file_size: 69 * i,
                ffprobe_info: FfProbe::default(),
            })
            .collect();

        db.insert_batch(&files)?;
        db.insert_batch(&files)?;

        let actual = db.list()?;
        assert_eq!(100, actual.len());

        Ok(())
    }

    #[test]
    fn test_insert_duplicate_path() -> Result<()> {
        let db = Database::in_memory()?;

        db.insert(NewTranscodeFile {
            path: "/1.mp4".into(),
            file_size: 5,
            ffprobe_info: FfProbe::default(),
        })?;

        let error = db.insert(NewTranscodeFile {
            path: "/1.mp4".into(),
            file_size: 5,
            ffprobe_info: FfProbe::default(),
        });

        assert!(error.is_err());
        let err = error.unwrap_err();
        assert!(err.to_string().contains("UNIQUE constraint failed"));

        Ok(())
    }

    #[test]
    fn test_ffprobe_info() -> Result<()> {
        let db = Database::in_memory()?;
        let ffprobe = ffprobe("./samples/claire.mp4")?;

        let file = NewTranscodeFile {
            path: "./samples/claire.mp4".into(),
            file_size: 130 * 1000 * 1000,
            ffprobe_info: ffprobe.clone(),
        };
        db.insert(file)?;
        let rows = db.list()?;
        assert_eq!(1, rows.len());
        assert!(rows[0].ffprobe().is_some());

        Ok(())
    }
}
