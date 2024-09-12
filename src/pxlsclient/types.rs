//! Partial definition of types used in the response for stats.json

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserRank {
    pub username: String,
    pub pixels: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct TopList {
    pub(super) canvas: Vec<UserRank>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PxlsResponse {
    pub(super) toplist: TopList,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PxlsPixelResponse {
    pub username: String,
    #[serde(rename = "pixelCount")]
    pub pixel_count: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserProfile {
    pub discord_tag: Option<String>,
    pub faction_id: Option<u64>,
}

#[derive(Default, Clone, Debug)]
pub struct UserProfileBuilder {
    discord_tag: Option<String>,
    faction_id: Option<u64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Pixel {
    pub x: u64,
    pub y: u64
}

#[derive(Deserialize, Debug, Clone)]
pub struct PixelUpdate {
    pub pixels: Vec<Pixel>,
}

impl UserProfileBuilder {
    pub fn discord_tag(mut self, discord_tag: String) -> Self {
        self.discord_tag = Some(discord_tag);
        self
    }

    pub fn faction_id(mut self, faction_id: u64) -> Self {
        self.faction_id = Some(faction_id);
        self
    }

    pub fn build(self) -> Result<UserProfile, anyhow::Error> {
        Ok(UserProfile {
            discord_tag: self.discord_tag,
            faction_id: self.faction_id,
        })
    }
}
