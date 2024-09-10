use serde::Serialize;

use crate::pxlsclient::UserRank;

#[derive(Serialize, Debug, Clone)]
pub struct UserRankResponse {
    pub user: UserRank,
    pub diff: u64
}
