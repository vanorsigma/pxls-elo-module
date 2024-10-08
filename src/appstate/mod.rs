use std::{env, sync::Arc};

use tokio::sync::Mutex;

use crate::{
    commandprocessor::{CommandProcessor, CommandProcessorImpl},
    database::{Database, DatabaseConnection, DatabaseConnectionCreater},
    pxlsclient::{PxlsClient, PxlsReqwestClient, WsHandlerImpl},
    pxlstemplateclient::PxlsTemplateParameters,
    rentryclient::{RentryClient, RentryClientImpl},
};

#[cfg(test)]
use crate::pxlsclient::MockPxlsClient;

pub struct AppState<
    D: Database + Send,
    P: PxlsClient + Send,
    R: RentryClient + Send,
    C: CommandProcessor<D, P>,
> {
    pub database: Arc<Mutex<D>>,
    pub pxlsclient: Arc<Mutex<P>>,
    pub cmdprocessor: Arc<Mutex<Option<C>>>,
    pub rentry: Arc<Mutex<R>>,
    pub parameters: Arc<Mutex<Option<PxlsTemplateParameters>>>,
    pub ignore_factions_list: Vec<u64>,
}

impl<D: Database + Send, P: PxlsClient + Send, R: RentryClient + Send, C: CommandProcessor<D, P>> Clone
    for AppState<D, P, R, C>
{
    fn clone(&self) -> Self {
        Self {
            database: self.database.clone(),
            pxlsclient: self.pxlsclient.clone(),
            cmdprocessor: self.cmdprocessor.clone(),
            rentry: self.rentry.clone(),
            parameters: self.parameters.clone(),
            ignore_factions_list: self.ignore_factions_list.clone(),
        }
    }
}

pub fn new_real_appstate(ignore_factions_list: Vec<u64>) -> Result<
    AppState<
        DatabaseConnection,
        PxlsReqwestClient,
        RentryClientImpl,
        CommandProcessorImpl<DatabaseConnection, PxlsReqwestClient, WsHandlerImpl, RentryClientImpl>,
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
        rentry: Arc::new(Mutex::new(RentryClientImpl::default())),
        cmdprocessor: Arc::new(Mutex::new(None)),
        parameters: Arc::new(Mutex::new(None)),
        ignore_factions_list,
    })
}

#[cfg(test)]
pub fn new_memory_appstate(ignore_factions_list: Vec<u64>) -> Result<
    AppState<
        DatabaseConnection,
        PxlsReqwestClient,
        RentryClientImpl,
        CommandProcessorImpl<DatabaseConnection, PxlsReqwestClient, WsHandlerImpl, RentryClientImpl>,
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
        rentry: Arc::new(Mutex::new(RentryClientImpl::default())),
        cmdprocessor: Arc::new(Mutex::new(None)),
        parameters: Arc::new(Mutex::new(None)),
        ignore_factions_list,
    })
}

#[cfg(test)]
use crate::pxlsclient::MockWsHandler;

#[cfg(test)]
use crate::rentryclient::MockRentryClientImpl;

#[cfg(test)]
pub fn new_testing_appstate() -> Result<
    AppState<
        DatabaseConnection,
        MockPxlsClient,
        MockRentryClientImpl,
        CommandProcessorImpl<DatabaseConnection, MockPxlsClient, MockWsHandler, MockRentryClientImpl>,
    >,
    anyhow::Error,
> {
    use crate::rentryclient::MockRentryClientImpl;

    Ok(AppState {
        database: Arc::new(Mutex::new(
            DatabaseConnectionCreater::open_in_memory().start()?,
        )),
        pxlsclient: Arc::new(Mutex::new(MockPxlsClient::default())),
        rentry: Arc::new(Mutex::new(MockRentryClientImpl::default())),
        cmdprocessor: Arc::new(Mutex::new(None)),
        parameters: Arc::new(Mutex::new(None)),
        ignore_factions_list: vec![],
    })
}
