#![feature(duration_constructors)]
use std::sync::Arc;

use pxls_elo_module::appstate::AppState;
use pxls_elo_module::commandprocessor;
use pxls_elo_module::commandprocessor::CommandProcessor;
use pxls_elo_module::database::Database;
use pxls_elo_module::pxlsclient::PxlsClient;
use pxls_elo_module::pxlstemplateclient::PxlsTemplateClient;
use pxls_elo_module::{appstate, pxlstemplateclient};
use tokio::sync::Mutex;

async fn callback_get_statistics<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    C: CommandProcessor<D, P>,
>(
    appstate: Arc<Mutex<AppState<D, P, C>>>,
) {
    let appstate_lock_guard = appstate.lock().await;
    let cmd_processor_lock_guard = appstate_lock_guard.cmdprocessor.lock().await;
    let cmd_processor = cmd_processor_lock_guard.as_ref().unwrap();
    cmd_processor.queue_command(commandprocessor::Command::UpdateStatistics);
}

async fn callback_get_factions<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    C: CommandProcessor<D, P>,
>(
    appstate: Arc<Mutex<AppState<D, P, C>>>,
) {
    let appstate_lock_guard = appstate.lock().await;
    let cmd_processor_lock_guard = appstate_lock_guard.cmdprocessor.lock().await;
    let cmd_processor = cmd_processor_lock_guard.as_ref().unwrap();
    let database = appstate_lock_guard.database.lock().await;

    database
        .get_all_users()
        .unwrap()
        .into_iter()
        .for_each(|record| {
            cmd_processor.queue_command(commandprocessor::Command::UpdateFaction(
                record.pxls_username,
            ))
        });
}

async fn callback_update_template<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    C: CommandProcessor<D, P>,
>(
    appstate: Arc<Mutex<AppState<D, P, C>>>,
) {
    let template_url = match std::env::var("PXLS_CANVAS_TEMPLATE") {
        Ok(s) => s,
        Err(_) => {
            log::info!("No canvas template env variable, not updating template");
            return;
        }
    };

    let template_client = pxlstemplateclient::PxlsTemplateClientImpl::default();
    let parameters = template_client
        .get_template_parameters_from_url(&template_url)
        .await
        .expect("can get template");

    {
        let app_state_lock = appstate.lock().await;
        app_state_lock.parameters.lock().await.replace(parameters);
    }
}

async fn callback_print_queue_length<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    C: CommandProcessor<D, P>,
>(
    appstate: Arc<Mutex<AppState<D, P, C>>>,
) {
    let app_state_lock = appstate.lock().await;
    let cmd_processor = app_state_lock.cmdprocessor.lock().await;
    cmd_processor
        .as_ref()
        .map(|processor| log::info!("Queue length currently at {}", processor.queue_len()));
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let app_state = Arc::new(Mutex::new(
        appstate::new_real_appstate().expect("can create normal app state"),
    ));

    let ws_client = {
        app_state
            .lock()
            .await
            .pxlsclient
            .lock()
            .await
            .get_websocket_handler()
            .await
            .unwrap()
    };

    commandprocessor::spawn_command_processor(app_state.clone(), ws_client).await;

    let mut recv = {
        let app_state_guard = app_state.lock().await;
        let cmdprocessor_guard = app_state_guard.cmdprocessor.lock().await;
        let cmdprocessor = cmdprocessor_guard.as_ref().unwrap();

        cmdprocessor.resubscribe()
    };

    let mut timer_1 = tokio::time::interval(tokio::time::Duration::from_mins(30));
    let mut timer_2 = tokio::time::interval(tokio::time::Duration::from_hours(12));
    let mut timer_3 = tokio::time::interval(tokio::time::Duration::from_mins(30));
    let mut timer_4 = tokio::time::interval(tokio::time::Duration::from_secs(5));

    timer_2.tick().await; // don't want to update factions immediately

    log::info!("Now entering main event loop");
    loop {
        let mut_receiver = &mut recv;

        tokio::select! {
            _ = timer_1.tick() => {
                callback_get_statistics(app_state.clone()).await;
            }

            _ = timer_2.tick() => {
                callback_get_factions(app_state.clone()).await;
            }

            _ = timer_3.tick() => {
                callback_update_template(app_state.clone()).await;
            }

            _ = timer_4.tick() => {
                callback_print_queue_length(app_state.clone()).await;
            }

            Ok(response) = mut_receiver.recv() => {
                match response {
                    commandprocessor::CommandResponse::UpdateStatisticsResponse(r) => {
                        println!("{}", serde_json::to_string(&r).unwrap())
                    },
                    _ => continue
                }
            }
        }
    }
}
