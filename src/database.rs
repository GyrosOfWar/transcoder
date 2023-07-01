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

    #[cfg(test)]
    pub fn with_file(path: &str) -> Result<Self> {
        let connection = Connection::open(path)?;
        Ok(Self { connection })
    }

    pub fn create_tables(&self) -> Result<()> {
        let sql = include_str!("../init.sql");
        self.connection.execute_batch(sql)?;
        Ok(())
    }

    pub fn insert_file(&self, file: &VideoFile) -> Result<i64> {
        let sql = "INSERT INTO video_files (path, duration, width, height, file_size, bitrate, framerate, codec) 
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT DO NOTHING
                   RETURNING rowid";
        let rowid: i64 = self.connection.query_row(
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
        Ok(rowid)
    }

    pub fn get_file(&self, id: i64) -> Result<VideoFile> {
        let sql = "SELECT rowid, * FROM video_files WHERE rowid = ?1";
        let result = self.connection.query_row(sql, params![id], |row| {
            Ok(VideoFile {
                rowid: Some(row.get(0)?),
                path: Utf8PathBuf::from(row.get::<_, String>(1)?),
                duration: row.get(2)?,
                resolution: (row.get(3)?, row.get(4)?),
                file_size: row.get(5)?,
                bitrate: row.get(6)?,
                frame_rate: row.get(7)?,
                codec: row.get(8)?,
            })
        })?;
        Ok(result)
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
        let id = db.insert_file(&file).unwrap();
        dbg!(id);
        let file = db.get_file(id).unwrap();
    }
}
