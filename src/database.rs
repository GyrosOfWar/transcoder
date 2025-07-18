use crate::{ffprobe::FfProbe, Result};
use camino::Utf8PathBuf;
use jiff::Timestamp;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_rows;
use tracing::info;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscodeStatus {
    Pending,
    Success,
    Error,
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
    pub ffprobe_info: Option<FfProbe>,
}

#[derive(Debug)]
pub struct NewTranscodeFile {
    pub path: Utf8PathBuf,
    pub file_size: u64,
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
        let connection = self.db.get()?;
        let mut statement = connection.prepare("SELECT rowid, * FROM transcode_files")?;
        let res = from_rows::<TranscodeFile>(statement.query([])?);
        let rows: Result<_, serde_rusqlite::Error> = res.collect();
        Ok(rows?)
    }

    pub fn insert_batch(&self, files: &[NewTranscodeFile]) -> Result<()> {
        info!("inserting batch of {} files", files.len());
        let mut connection = self.db.get()?;

        let now = Timestamp::now().as_second();
        let tx = connection.transaction()?;
        {
            let mut statement = tx.prepare("INSERT INTO transcode_files (path, created_on, updated_on, file_size) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (path) DO NOTHING")?;
            for file in files {
                statement.execute(params![file.path.as_str(), now, now, file.file_size as i64,])?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    pub fn set_ffprobe_info(&self, rowid: i64, info: &FfProbe) -> Result<()> {
        let connection = self.db.get()?;
        let json_info = serde_json::to_string(info)?;
        connection.execute(
            "UPDATE transcode_files SET ffprobe_info = ?1 WHERE rowid = ?2",
            params![json_info, rowid],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_row() -> Result<()> {
        let db = Database::in_memory()?;

        db.insert(NewTranscodeFile {
            path: "/stuff/1.mp4".into(),
            file_size: 696969,
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
        })?;

        let error = db.insert(NewTranscodeFile {
            path: "/1.mp4".into(),
            file_size: 5,
        });

        assert!(error.is_err());
        let err = error.unwrap_err();
        assert!(err.to_string().contains("UNIQUE constraint failed"));

        Ok(())
    }
}
