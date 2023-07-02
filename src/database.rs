use std::result::Result as StdResult;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use parking_lot::Mutex;
use rusqlite::{params, Connection, Row};

use crate::Result;

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub rowid: Option<i64>,
    pub path: Utf8PathBuf,
    pub duration: f64,
    pub resolution: (u32, u32),
    pub bitrate: u64,
    pub frame_rate: f64,
    pub codec: String,
    pub file_size: u64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl<'a> TryFrom<&'a Row<'a>> for VideoFile {
    type Error = rusqlite::Error;

    fn try_from(row: &'a Row<'a>) -> StdResult<Self, Self::Error> {
        Ok(Self {
            rowid: row.get(0)?,
            path: Utf8PathBuf::from(row.get::<_, String>(1)?),
            duration: row.get(2)?,
            resolution: (row.get(3)?, row.get(4)?),
            bitrate: row.get(5)?,
            file_size: row.get(6)?,
            frame_rate: row.get(7)?,
            codec: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }
}

pub struct VideoConversion {
    pub video_file_id: i64,
    pub original_codec: String,
    pub new_codec: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new() -> Result<Self> {
        let connection = Connection::open("./transcoder.sqlite3")?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    #[cfg(test)]
    pub fn test() -> Result<Self> {
        let path = "test.sqlite3";
        if Utf8Path::new(path).exists() {
            std::fs::remove_file(path)?;
        }
        let connection = Connection::open(path)?;
        let db = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        db.create_tables()?;
        Ok(db)
    }

    pub fn create_tables(&self) -> Result<()> {
        let connection = self.connection.lock();
        let sql = include_str!("../init.sql");
        connection.execute_batch(sql)?;
        Ok(())
    }

    #[allow(unused)]
    pub fn file_exists(&self, path: &str) -> Result<bool> {
        let sql = "SELECT EXISTS(SELECT 1 FROM video_files WHERE path = ?1 LIMIT 1)";
        let connection = self.connection.lock();
        let result: i64 = connection.query_row(sql, params![path], |row| row.get(0))?;
        Ok(result == 1)
    }

    pub fn insert_file(&self, file: &VideoFile) -> Result<VideoFile> {
        let sql = "INSERT INTO video_files (path, duration, width, height, file_size, bitrate, framerate, codec) 
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT (path) DO UPDATE SET updated_at = CURRENT_TIMESTAMP
                   RETURNING rowid";
        let connection = self.connection.lock();
        let rowid: i64 = connection.query_row(
            sql,
            params![
                file.path.as_str(),
                file.duration,
                file.resolution.0,
                file.resolution.1,
                file.file_size,
                file.bitrate,
                file.frame_rate,
                file.codec
            ],
            |row| row.get(0),
        )?;
        Ok(VideoFile {
            rowid: Some(rowid),
            ..file.clone()
        })
    }

    #[allow(unused)]
    pub fn get_file(&self, id: i64) -> Result<VideoFile> {
        let sql = "SELECT rowid, * FROM video_files WHERE rowid = ?1";
        let connection = self.connection.lock();
        let result = connection.query_row(sql, params![id], |row| VideoFile::try_from(row))?;
        Ok(result)
    }

    pub fn get_files_with_path_prefix(&self, path: impl AsRef<Utf8Path>) -> Result<Vec<VideoFile>> {
        let sql = "SELECT rowid, * FROM video_files WHERE path LIKE ?1";
        let connection = self.connection.lock();
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map(params![format!("{}%", path.as_ref().as_str())], |r| {
            VideoFile::try_from(r)
        })?;

        let mut files = vec![];
        for row in rows {
            files.push(row?);
        }
        Ok(files)
    }

    pub fn add_video_conversion(&self, video: VideoConversion) -> Result<()> {
        let sql = "INSERT INTO video_conversions (video_file_id, original_codec, new_codec) 
                   VALUES (?1, ?2, ?3)";
        let connection = self.connection.lock();
        connection.execute(
            sql,
            params![video.video_file_id, video.original_codec, video.new_codec],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use crate::database::{Database, VideoFile};

    #[test]
    fn test_insert_file() {
        let db = Database::test().unwrap();

        let file = VideoFile {
            rowid: None,
            path: Utf8PathBuf::from("test.mp4"),
            duration: 100.0,
            resolution: (1920, 1080),
            bitrate: 5000,
            frame_rate: 30.0,
            codec: "h264".to_string(),
            file_size: 1000000,
            created_at: None,
            updated_at: None,
        };
        let id = db.insert_file(&file).unwrap().rowid.unwrap();
        let file = db.get_file(id).unwrap();
        assert_eq!(file.rowid, Some(id));
        assert!(file.created_at.is_some());
        assert!(file.updated_at.is_some());
    }
}
