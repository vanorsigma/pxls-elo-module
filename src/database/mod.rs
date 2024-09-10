use anyhow::anyhow;
use types::UserRecord;

pub mod types;

pub trait Database {
    // Gets a user record from the DB
    fn get_user_record(&self, username: &str) -> Result<UserRecord, anyhow::Error>;

    // Insert a user record into the DB
    fn insert_user_record(&self, record: UserRecord) -> Result<(), anyhow::Error>;

    // Gets a list of all users from the DB
    fn get_all_users(&self) -> Result<Vec<UserRecord>, anyhow::Error>;
}

#[derive(Default, Debug)]
pub struct DatabaseConnectionCreater {
    file_path: Option<String>,
}

pub struct DatabaseConnection {
    connection: rusqlite::Connection,
}

impl DatabaseConnectionCreater {
    pub fn open_else_new(file_path: &str) -> Self {
        Self {
            file_path: Some(file_path.to_string()),
        }
    }

    pub fn open_in_memory() -> Self {
        Self::default()
    }

    fn initialize_table_if_needed(
        dbconn: DatabaseConnection,
    ) -> Result<DatabaseConnection, anyhow::Error> {
        let statement = r#"
CREATE TABLE IF NOT EXISTS "users" (
  "PXLS_USERNAME"	TEXT,
  "DISCORD_TAG"	TEXT,
  "FACTION"     INTEGER,
  "SCORE"	INTEGER,
  "DIRTY_STATE"	INTEGER DEFAULT 0,
  PRIMARY KEY("PXLS_USERNAME")
);
"#;
        let _ = dbconn.connection.execute(statement, ())?;
        Ok(dbconn)
    }

    pub fn start(self) -> Result<DatabaseConnection, anyhow::Error> {
        let connection = match self.file_path {
            Some(path) => DatabaseConnection {
                connection: rusqlite::Connection::open(path)?,
            },
            None => DatabaseConnection {
                connection: rusqlite::Connection::open_in_memory()?,
            },
        };

        Ok(Self::initialize_table_if_needed(connection)?)
    }
}

impl Database for DatabaseConnection {
    fn get_user_record(&self, username: &str) -> Result<UserRecord, anyhow::Error> {
        let mut statement = self.connection.prepare(r#"SELECT PXLS_USERNAME, DISCORD_TAG, FACTION, SCORE, DIRTY_STATE FROM users WHERE PXLS_USERNAME = ? LIMIT 1;"#)?;
        let x = statement
            .query_map((username,), |row| {
                Ok(UserRecord {
                    pxls_username: row.get(0)?,
                    discord_tag: row.get(1)?,
                    faction: row.get(2)?,
                    score: row.get(3)?,
                    dirty_slate: row.get(4)?,
                })
            })?
            .filter_map(|record| record.ok())
            .collect::<Vec<_>>()
            .get(0)
            .cloned()
            .ok_or(anyhow!("record not found"));
        x
    }

    fn insert_user_record(&self, record: UserRecord) -> Result<(), anyhow::Error> {
        let statement = r#"INSERT OR REPLACE INTO users (PXLS_USERNAME, DISCORD_TAG, FACTION, SCORE, DIRTY_STATE) VALUES (?, ?, ?, ?, ?);"#;

        let _ = self.connection.execute(
            statement,
            (
                record.pxls_username,
                record.discord_tag,
                record.faction,
                record.score,
                record.dirty_slate,
            ),
        )?;

        Ok(())
    }

    fn get_all_users(&self) -> Result<Vec<UserRecord>, anyhow::Error> {
        let mut statement = self.connection.prepare(
            r#"SELECT PXLS_USERNAME, DISCORD_TAG, FACTION, SCORE, DIRTY_STATE FROM users"#,
        )?;
        let x = statement
            .query_map((), |row| {
                Ok(UserRecord {
                    pxls_username: row.get(0)?,
                    discord_tag: row.get(1)?,
                    faction: row.get(2)?,
                    score: row.get(3)?,
                    dirty_slate: row.get(4)?,
                })
            })?
            .filter_map(|record| record.ok())
            .collect::<Vec<_>>();

        Ok(x)
    }
}

#[cfg(test)]
mod tests {
    use super::{Database, DatabaseConnectionCreater};

    #[test]
    fn test_gets_record_after_insertion() {
        let creater = DatabaseConnectionCreater::open_in_memory();
        let dbconn = creater.start().expect("can open in memory db");
        let _ = dbconn.insert_user_record(crate::database::types::UserRecord {
            pxls_username: "vanorsigma".to_string(),
            faction: Some(456),
            discord_tag: Some("vanorsigma".to_string()),
            score: Some(123),
            dirty_slate: Some(0),
        });

        let record = dbconn
            .get_user_record("vanorsigma")
            .expect("can get user record");
        assert_eq!(record.pxls_username, "vanorsigma");
        assert_eq!(record.discord_tag, Some("vanorsigma".to_string()));
        assert_eq!(record.faction, Some(456));
        assert_eq!(record.score, Some(123));
        assert_eq!(record.dirty_slate, Some(0));
    }

    #[test]
    fn test_gets_records_after_insertion() {
        let creater = DatabaseConnectionCreater::open_in_memory();
        let dbconn = creater.start().expect("can open in memory db");
        let _ = dbconn.insert_user_record(crate::database::types::UserRecord {
            pxls_username: "vanorsigma".to_string(),
            faction: Some(456),
            discord_tag: Some("vanorsigma".to_string()),
            score: Some(123),
            dirty_slate: Some(0),
        });

        let _ = dbconn.insert_user_record(crate::database::types::UserRecord {
            pxls_username: "owobred".to_string(),
            faction: Some(789),
            discord_tag: Some("owobred".to_string()),
            score: Some(312),
            dirty_slate: Some(0),
        });

        let records = dbconn.get_all_users().expect("can get a list of all users");
        assert_eq!(records[0].pxls_username, "vanorsigma");
        assert_eq!(records[1].pxls_username, "owobred");
    }
}
