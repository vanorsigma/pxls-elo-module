#[derive(Clone)]
pub struct UserRecord {
    pub pxls_username: String,
    pub discord_tag: Option<String>,
    pub faction: Option<u64>,
    pub score: Option<u64>,
    pub dirty_slate: Option<u64>,
}
