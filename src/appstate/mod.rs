use std::{env, sync::Arc};

use tokio::sync::Mutex;

use crate::{
    commandprocessor::{CommandProcessor, CommandProcessorImpl},
    database::{Database, DatabaseConnection, DatabaseConnectionCreater},
    pxlsclient::{PxlsClient, PxlsReqwestClient},
};

#[cfg(test)]
use crate::pxlsclient::MockPxlsClient;

pub struct AppState<D: Database + Send, P: PxlsClient + Send, C: CommandProcessor<D, P>> {
    pub database: Arc<Mutex<D>>,
    pub(super) pxlsclient: Arc<Mutex<P>>,
    pub cmdprocessor: Arc<Mutex<Option<C>>>,
}

impl<D: Database + Send, P: PxlsClient + Send, C: CommandProcessor<D, P>> Clone
    for AppState<D, P, C>
{
    fn clone(&self) -> Self {
        Self {
            database: self.database.clone(),
            pxlsclient: self.pxlsclient.clone(),
            cmdprocessor: self.cmdprocessor.clone(),
        }
    }
}

pub fn new_real_appstate() -> Result<
    AppState<
        DatabaseConnection,
        PxlsReqwestClient,
        CommandProcessorImpl<DatabaseConnection, PxlsReqwestClient>,
    >,
    anyhow::Error,
> {
    let pxls_auth_token = env::var("PXLS_AUTH_TOKEN").expect("please set PXLS_AUTH_TOKEN");
    let pxls_cf_token = env::var("PXLS_CF_TOKEN").expect("please set PXLS_CF_TOKEN");

    Ok(AppState {
        database: Arc::new(Mutex::new(
            DatabaseConnectionCreater::open_else_new("local.db").start()?,
        )),
        pxlsclient: Arc::new(Mutex::new(PxlsReqwestClient::new_with_token(
            &pxls_auth_token,
            &pxls_cf_token,
        ))),
        cmdprocessor: Arc::new(Mutex::new(None)),
    })
}

#[cfg(test)]
pub fn new_memory_appstate() -> Result<
    AppState<
        DatabaseConnection,
        PxlsReqwestClient,
        CommandProcessorImpl<DatabaseConnection, PxlsReqwestClient>,
    >,
    anyhow::Error,
> {
    let pxls_auth_token = env::var("PXLS_AUTH_TOKEN").expect("please set PXLS_AUTH_TOKEN");
    let pxls_cf_token = env::var("PXLS_CF_TOKEN").expect("please set PXLS_CF_TOKEN");

    Ok(AppState {
        database: Arc::new(Mutex::new(
            DatabaseConnectionCreater::open_in_memory().start()?,
        )),
        pxlsclient: Arc::new(Mutex::new(PxlsReqwestClient::new_with_token(
            &pxls_auth_token,
            &pxls_cf_token,
        ))),
        cmdprocessor: Arc::new(Mutex::new(None)),
    })
}

#[cfg(test)]
pub fn new_testing_appstate() -> Result<
    AppState<
        DatabaseConnection,
        MockPxlsClient,
        CommandProcessorImpl<DatabaseConnection, MockPxlsClient>,
    >,
    anyhow::Error,
> {
    Ok(AppState {
        database: Arc::new(Mutex::new(
            DatabaseConnectionCreater::open_in_memory().start()?,
        )),
        pxlsclient: Arc::new(Mutex::new(MockPxlsClient::default())),
        cmdprocessor: Arc::new(Mutex::new(None)),
    })
}
