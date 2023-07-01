use camino::Utf8PathBuf;
use rusqlite::{params, Connection};

use crate::Result;

#[derive(Debug)]
pub struct VideoFile {
    pub rowid: Option<i64>,
    pub path: Utf8PathBuf,
    pub duration: f64,
    pub resolution: (u32, u32),
    pub bitrate: u64,
    pub frame_rate: f64,
    pub codec: String,
    pub file_size: u64,
}

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn new() -> Result<Self> {
        let connection = Connection::open("./transcoder.sqlite3")?;
        Ok(Self { connection })
    }

    pub fn with_file(path: &str) -> Result<Self> {
        let connection = Connection::open(path)?;
        Ok(Self { connection })
    }

    pub fn create_tables(&self) -> Result<()> {
        let sql = include_str!("../init.sql");
        self.connection.execute_batch(sql)?;
        Ok(())
    }

    pub fn insert_file(&self, file: &VideoFile) -> Result<()> {
        let sql = "INSERT INTO files (path, duration, width, height, file_size, bitrate, framerate, codec) 
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT DO NOTHING";
        self.connection.execute(
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
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    #[test]
    fn test_insert_file() {
        use crate::database::{Database, VideoFile};

        let db = Database::with_file("test.sqlite3").unwrap();
        db.create_tables().unwrap();
        let file = VideoFile {
            rowid: None,
            path: Utf8PathBuf::from("test.mp4"),
            duration: 100.0,
            resolution: (1920, 1080),
            bitrate: 1000000,
            frame_rate: 30.0,
            codec: "h264".to_string(),
            file_size: 1000000,
        };
        db.insert_file(&file).unwrap();
    }
}
