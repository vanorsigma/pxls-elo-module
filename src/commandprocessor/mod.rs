use core::time;
use std::{
    collections::VecDeque,
    sync::{atomic::AtomicBool, Arc},
};

mod types;

use crate::{
    appstate::AppState,
    database::{types::UserRecord, Database},
    pxlsclient::{PixelUpdate, PxlsClient, PxlsPixelResponse, UserRank, WsHandler},
    pxlstemplateclient::FilledPixelCoordinatable,
    rentryclient::RentryClient,
};
use anyhow::anyhow;
use tokio::{sync::Mutex, task::JoinHandle};
use types::UserRankResponse;

pub enum Command {
    UpdateFromWebSocket(PixelUpdate),
    UpdateFromWebSocketAfterMetadata(PxlsPixelResponse),
    UpdateStatistics,
    UpdateFaction(String),
    UpdateScore(String),
    UpdateUsersFromURL(String, u64),
}

#[non_exhaustive]
#[derive(Clone)]
pub enum CommandResponse {
    UpdateStatisticsResponse(Vec<UserRankResponse>),
}

pub trait CommandProcessor<D: Database, P: PxlsClient> {
    type ChannelType;

    /// Adds a command to the queue
    fn queue_command(&self, cmd: Command);

    /// Gets the length of the queue. This must be also be callable in an async context
    fn queue_len(&self) -> usize;

    /// Gets a subscriber to the results from any of the commands
    fn resubscribe(&self) -> Self::ChannelType;
}

/// Internal implementation, used to spawn a runaway thread by the CommandProcessorCreator
pub trait CommandProcessorInternal<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    W: WsHandler + Send + 'static,
    R: RentryClient + Send + 'static,
    C: CommandProcessor<D, P>,
>
{
    /// This function will be called in a loop in a blocking task.
    /// Rate-limiting should be done by the implementation.
    /// If this function returns true, do_once will be called again. Else, the task will terminate
    fn do_once(&mut self) -> impl std::future::Future<Output = bool> + std::marker::Send;

    /// Creates Self, injecting the handle into the CommandProcessor
    /// It is the implementers responsibility to cause do_once to return false once the implementor is dropped
    fn inject_command_processor_into_appstate(
        app_state: Arc<Mutex<AppState<D, P, R, C>>>,
        ws_handler: W,
        handle: JoinHandle<()>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

pub struct CommandProcessorImpl<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    W: WsHandler + Send + 'static,
    R: RentryClient + Send + 'static,
> {
    app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
    queue: Arc<Mutex<VecDeque<Command>>>,
    tx: tokio::sync::broadcast::Sender<CommandResponse>,
    rx: tokio::sync::broadcast::Receiver<CommandResponse>,
    #[allow(dead_code)]
    queue_handle: JoinHandle<()>,

    ws: W,

    last_once: AtomicBool, // used to signal a drop
}

fn extract_result_vec_error<T, E>(vector: Vec<Result<T, E>>) -> Result<Vec<T>, E> {
    let mut result_without_error = vec![];
    for item in vector {
        result_without_error.push(item?);
    }
    Ok(result_without_error)
}

impl<
        D: Database + Send,
        P: PxlsClient + Send,
        W: WsHandler + Send + Sync,
        R: RentryClient + Send,
    > CommandProcessorInternal<D, P, W, R, Self> for CommandProcessorImpl<D, P, W, R>
{
    async fn do_once(&mut self) -> bool {
        if self.last_once.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }

        if let Some(cmd) = self.queue.lock().await.pop_front() {
            self.process_command(cmd)
        }

        while let Ok(pixel_update) = self.ws.try_recv().await {
            self.queue
                .lock()
                .await
                .push_back(Command::UpdateFromWebSocket(pixel_update));
        }

        true
    }

    async fn inject_command_processor_into_appstate(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
        ws: W,
        handle: JoinHandle<()>,
    ) {
        let (tx, rx) = tokio::sync::broadcast::channel(10);

        let ret_val = Self {
            app_state: app_state.clone(),
            tx,
            rx,
            ws,
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            queue_handle: handle,
            last_once: false.into(),
        };
        app_state
            .lock()
            .await
            .cmdprocessor
            .lock()
            .await
            .replace(ret_val);
    }
}

impl<D: Database + Send, P: PxlsClient + Send, W: WsHandler + Send, R: RentryClient + Send> Drop
    for CommandProcessorImpl<D, P, W, R>
{
    fn drop(&mut self) {
        self.last_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // leave the handle alone to terminate
    }
}

impl<D: Database + Send, P: PxlsClient + Send, W: WsHandler + Send, R: RentryClient + Send>
    CommandProcessorImpl<D, P, W, R>
{
    fn update_statistic_for(
        cmdprocessor: Option<&Self>,
        database: &D,
        username: &str,
        pixels: u64,
    ) -> Result<UserRankResponse, anyhow::Error> {
        let record = database.get_user_record(username);
        let record_exists = record.is_ok();

        log::debug!("Record for {} exists? {}", username, record_exists);

        match record {
            Ok(mut r) => {
                log::debug!("Inserting updated pixel counts");
                r.pxls_username = username.to_string();
                r.score = Some(pixels);
                database.insert_user_record(r)?;
            }

            Err(_) => database
                .insert_user_record(UserRecord {
                    pxls_username: username.to_string(),
                    discord_tag: None,
                    faction: None,
                    score: Some(pixels),
                    dirty_slate: Some(0),
                })
                .inspect(|_| {
                    log::debug!("Queueing the UpdateFaction command");
                    cmdprocessor.inspect(|processor| {
                        processor.queue_command(Command::UpdateFaction(username.to_string()));
                    });
                })?,
        };

        let new_record = database
            .get_user_record(username)
            .map_err(|e| anyhow!("cannot get new record, {}", e))?;

        Ok(UserRankResponse {
            user: UserRank {
                username: username.to_string(),
                pixels,
            },
            diff: if !record_exists {
                pixels
            } else if pixels > 0 {
                pixels - new_record.score.unwrap()
            } else {
                0
            },
        })
    }

    async fn update_statistics(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
    ) -> Result<Vec<UserRankResponse>, anyhow::Error> {
        log::debug!("Updating statistics!");
        let user_ranks = {
            app_state
                .lock()
                .await
                .pxlsclient
                .lock()
                .await
                .get_canvas_user_ranks()
                .await?
        };

        log::debug!("Statistics is acquiring a series of locks...");
        let app_state_unlocked = app_state.lock().await;
        let database = app_state_unlocked.database.lock().await;
        let cmdprocessor = app_state_unlocked.cmdprocessor.lock().await;
        log::debug!("Statistics acquired the series of locks");

        // intentional implementation, otherwise we'll need to capture
        // async move, which the executor may process fairly slowly
        let user_rank_responses = user_ranks
            .into_iter()
            .map(|rank| {
                Self::update_statistic_for(
                    cmdprocessor.as_ref(),
                    &database,
                    &rank.username,
                    rank.pixels,
                )
            })
            .collect::<Vec<_>>();

        extract_result_vec_error(user_rank_responses)
    }

    async fn update_from_websocket(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
        pixel_update: PixelUpdate,
    ) {
        for pixel in pixel_update.pixels {
            log::debug!(
                "Update from websockets is acquiring a series of locks for x: {}, y: {}... ",
                pixel.x,
                pixel.y
            );
            let app_state_unlocked = app_state.lock().await;
            let cmdprocessor = app_state_unlocked.cmdprocessor.lock().await;
            let pxlsclient = app_state_unlocked.pxlsclient.lock().await;
            let parameters = app_state_unlocked.parameters.lock().await;
            log::debug!("Update from websockets acquired the series of locks");

            let filled_squares = match parameters.as_ref() {
                Some(param) => param.get_filled_pixel_coordinates(),
                None => continue,
            };

            if !filled_squares.contains(&(pixel.x as u32, pixel.y as u32)) {
                continue;
            }

            log::debug!("Requesting pixel data for {}, {}", pixel.x, pixel.y);
            let _ = pxlsclient
                .get_metadata_for_pixel(pixel.x, pixel.y)
                .await
                .inspect_err(|e| {
                    log::warn!("cannot get metadata for pixel: {:#?} with {}", pixel, e)
                })
                .map(|metadata| {
                    cmdprocessor.as_ref().map(|processor| {
                        processor
                            .queue_command(Command::UpdateFromWebSocketAfterMetadata(metadata));
                    });
                });

            // rudimentary rate-limiting
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            drop(parameters);
            drop(pxlsclient);
            drop(cmdprocessor);
            drop(app_state_unlocked);
        }
    }

    async fn update_from_websocket_lookup_response(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
        response: PxlsPixelResponse,
    ) -> Result<UserRankResponse, anyhow::Error> {
        log::debug!(
            "Update from websockets after lookup response is acquiring a series of locks..."
        );
        let app_state_unlocked = app_state.lock().await;
        let database = app_state_unlocked.database.lock().await;
        let cmdprocessor = app_state_unlocked.cmdprocessor.lock().await;
        log::debug!("Update from websockets after lookup response acquired the series of locks");

        Self::update_statistic_for(
            cmdprocessor.as_ref(),
            &database,
            &response.username,
            response.pixel_count,
        )
    }

    async fn update_faction(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
        username: &str,
    ) -> Result<(), anyhow::Error> {
        log::debug!("Factions is acquiring a series of locks...");
        let app_state = app_state.lock().await;
        let pxlsclient = app_state.pxlsclient.lock().await;
        let database = app_state.database.lock().await;
        log::debug!("Factions has acquired locks");
        // NOTE: If we just called .await here it add an awaitable entry into the
        // queue (or something similar).

        // Because we don't know how the scheduler will queue these
        // awaits, it is possible for us to break the rate limit if
        // this happens: A acquire locks, B (1 second later) tries to
        // acquire locks but are stuck, A proceeds with request, B
        // acquires locks and immediately proceeds with
        // request. Rate-limit broken. So, we have to sleep here
        // instead, so that we maintain the locks.
        let profile = pxlsclient.get_profile_for_user(username).await?;
        log::debug!("Factions have acquired profile for {}", username);

        // rudimentary rate-limiting (TODO: possibly use a shared sleep / interval or something)
        tokio::time::sleep(time::Duration::from_secs(1)).await;

        let mut user = database.get_user_record(username)?;
        if let Some(faction) = user.faction {
            if !app_state.ignore_factions_list.contains(&faction) {
                user.faction = profile.faction_id;
            }
        }
        user.discord_tag = profile.discord_tag;
        user.score = profile.pixels;
        database.insert_user_record(user)?;

        Ok(())
    }

    /// This does the same thing as update_factions, but ONLY touches the scores
    async fn update_score(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
        username: &str,
    ) -> Result<(), anyhow::Error> {
        log::debug!("Score update is acquiring a series of locks...");
        let app_state = app_state.lock().await;
        let pxlsclient = app_state.pxlsclient.lock().await;
        let database = app_state.database.lock().await;
        log::debug!("Score has acquired locks");

        let profile = pxlsclient.get_profile_for_user(username).await?;
        log::debug!("Factions have acquired profile for {}", username);

        // rudimentary rate-limiting (TODO: possibly use a shared sleep / interval or something)
        tokio::time::sleep(time::Duration::from_millis(500)).await;

        let mut user = database.get_user_record(username)?;
        user.discord_tag = profile.discord_tag;
        user.score = profile.pixels;
        database.insert_user_record(user)?;

        Ok(())
    }

    async fn update_users_from_url(
        app_state: Arc<Mutex<AppState<D, P, R, Self>>>,
        url: &str,
        faction_id: u64,
    ) -> Result<(), anyhow::Error> {
        log::debug!("Update users acquring a series of locks...");
        let app_state_unlocked = app_state.lock().await;
        let database = app_state_unlocked.database.lock().await;
        let rentry = app_state_unlocked.rentry.lock().await;
        let cmdprocessor = app_state_unlocked.cmdprocessor.lock().await;

        log::debug!("Update users acquired locks");
        rentry
            .get_users_from_url(url)
            .await?
            .iter()
            .map(|entry| -> Result<_, anyhow::Error> {
                let record = match database.get_user_record(&entry) {
                    Ok(mut record) => {
                        record.faction = Some(faction_id);
                        record
                    }
                    Err(_) => UserRecord {
                        pxls_username: entry.to_string(),
                        discord_tag: None,
                        faction: Some(faction_id),
                        score: Some(0),
                        dirty_slate: Some(0),
                    },
                };

                database
                    .insert_user_record(record)
                    .map_err(|e| anyhow!(e))?;

                Ok(cmdprocessor
                    .as_ref()
                    .map(|processor| {
                        processor.queue_command(Command::UpdateScore(entry.to_string()))
                    })
                    .ok_or(anyhow!("no processor found"))?)
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;

        Ok(())
    }
}

impl<
        D: Database + Send + 'static,
        P: PxlsClient + Send + 'static,
        W: WsHandler + Send + 'static,
        R: RentryClient + Send + 'static,
    > CommandProcessorImpl<D, P, W, R>
{
    fn process_command(&self, cmd: Command) {
        let app_state = self.app_state.clone();
        let tx = self.tx.clone();
        match cmd {
            Command::UpdateFromWebSocket(value) => {
                log::debug!("Updating from websocket");
                tokio::task::spawn(
                    async move { Self::update_from_websocket(app_state, value).await },
                );
            }

            Command::UpdateFromWebSocketAfterMetadata(value) => {
                log::debug!("Updating from websocket after metadata request");
                tokio::task::spawn(async move {
                    match Self::update_from_websocket_lookup_response(app_state, value).await {
                        Ok(x) => {
                            log::debug!("Got a response for websocket after metadata command.");
                            if let Err(e) =
                                tx.send(CommandResponse::UpdateStatisticsResponse(vec![x]))
                            {
                                log::error!(
                                    "problem with sending the command response to channel {}",
                                    e
                                )
                            }
                        }
                        Err(e) => log::error!("problem with obtaining metadata {}", e),
                    }
                });
            }
            Command::UpdateStatistics => {
                log::debug!("Update statistics command...");
                tokio::task::spawn(async move {
                    match Self::update_statistics(app_state).await {
                        Ok(x) => {
                            log::debug!("Got a response for statistics command.");
                            if let Err(e) = tx.send(CommandResponse::UpdateStatisticsResponse(x)) {
                                log::error!(
                                    "problem with sending the command response to channel {}",
                                    e
                                )
                            }
                        }
                        Err(e) => log::error!("problem with obtaining statistics {}", e),
                    }
                });
            }
            Command::UpdateFaction(user) => {
                log::debug!("Update faction command...");
                tokio::task::spawn(async move {
                    match Self::update_faction(app_state, &user).await {
                        Ok(_) => {
                            log::info!("{}'s faction was updated", user);
                        }
                        Err(e) => {
                            log::error!("problem while updating faction for user {}", e)
                        }
                    }
                });
            }
            Command::UpdateScore(user) => {
                log::debug!("Update score command...");
                tokio::task::spawn(async move {
                    match Self::update_score(app_state, &user).await {
                        Ok(_) => {
                            log::info!("{}'s score was updated", user);
                        }
                        Err(e) => {
                            log::error!("problem while updating score {}", e)
                        }
                    }
                });
            }
            Command::UpdateUsersFromURL(url, faction_id) => {
                log::debug!("Update from URL");
                tokio::task::spawn(async move {
                    match Self::update_users_from_url(app_state, &url, faction_id).await {
                        Ok(_) => {
                            log::info!("Updated from URL");
                        }
                        Err(e) => {
                            log::error!("problem while processing update users from url {}", e)
                        }
                    }
                });
            }
        }
    }
}

impl<
        D: Database + Send + 'static,
        P: PxlsClient + Send + 'static,
        W: WsHandler + Send + 'static,
        R: RentryClient + Send + 'static,
    > CommandProcessor<D, P> for CommandProcessorImpl<D, P, W, R>
{
    type ChannelType = tokio::sync::broadcast::Receiver<CommandResponse>;

    fn resubscribe(&self) -> Self::ChannelType {
        self.rx.resubscribe()
    }

    fn queue_command(&self, cmd: Command) {
        tokio::task::block_in_place(|| self.queue.blocking_lock().push_back(cmd))
    }

    fn queue_len(&self) -> usize {
        tokio::task::block_in_place(|| self.queue.blocking_lock().len())
    }
}

pub async fn spawn_command_processor<
    D: Database + Send + 'static,
    P: PxlsClient + Send + 'static,
    W: WsHandler + Send + 'static,
    R: RentryClient + Send + 'static,
    C: CommandProcessor<D, P> + CommandProcessorInternal<D, P, W, R, C> + Send + 'static,
>(
    app_state: Arc<Mutex<AppState<D, P, R, C>>>,
    ws: W,
) {
    let app_state_arc = app_state.clone();
    let handle = tokio::task::spawn(async move {
        let temp_app_state_lock = app_state_arc.lock().await;
        let cmdprocessor_lock = temp_app_state_lock.cmdprocessor.clone();
        drop(temp_app_state_lock);

        // to ensure that we release the cmdprocessor lock correctly
        let mut continue_processing = true;

        while continue_processing {
            let mut cmd_processor = cmdprocessor_lock.lock().await;
            match cmd_processor.as_mut() {
                Some(processor) => continue_processing = processor.do_once().await,
                None => continue, // cmd_processor hasn't settled in yet
            }
            // lock is released here
        }
    });

    C::inject_command_processor_into_appstate(app_state, ws, handle).await;
}

#[cfg(test)]
mod tests {
    use crate::pxlsclient::PxlsClient;
    use std::{mem, sync::Arc, time::Duration};

    use anyhow::anyhow;
    use tokio::{sync::Mutex, time::timeout};

    use crate::{
        appstate::{new_memory_appstate, new_testing_appstate},
        commandprocessor::CommandProcessor,
        database::{types::UserRecord, Database},
        pxlsclient::{MockWsHandler, Pixel, PixelUpdate, PxlsPixelResponse, UserProfile, UserRank},
    };

    #[ignore = "this is a live test, requires actual credentials"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_real_update_statistics() {
        env_logger::init();
        let app_state = Arc::new(Mutex::new(
            new_memory_appstate(vec![]).expect("can create app state"),
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

        super::spawn_command_processor(app_state.clone(), ws_client).await;

        async move {
            // intentional closure
            let command_processor_arc = { app_state.lock().await.cmdprocessor.clone() };
            let cmd_proc_locked = command_processor_arc.lock().await;
            let cmd_proc = cmd_proc_locked.as_ref().unwrap();

            let mut rx = cmd_proc.resubscribe();
            cmd_proc.process_command(super::Command::UpdateStatistics);
            drop(cmd_proc_locked);

            timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("should have received something by 5 seconds")
                .expect("should have successfully received something");
        }
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_statistics_direct() {
        let testing_appstate = new_testing_appstate().expect("can create test appstate");
        {
            // with pxls lock
            let mut pxclient = testing_appstate.pxlsclient.lock().await;
            mem::swap(
                &mut pxclient.user_ranks_ret_val,
                &mut Ok(vec![UserRank {
                    username: "vanorsigma".to_string(),
                    pixels: 123,
                }]),
            );
            mem::swap(
                &mut pxclient.user_profiles_ret_val,
                &mut Err(anyhow!("intentional mock error")),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        super::spawn_command_processor(app_state.clone(), MockWsHandler::default()).await;

        let cloned_app_state = app_state.clone();
        async move {
            // intentional closure
            let command_processor_arc = { cloned_app_state.lock().await.cmdprocessor.clone() };
            let cmd_proc_locked = command_processor_arc.lock().await;
            let cmd_proc = cmd_proc_locked.as_ref().unwrap();

            let mut rx = cmd_proc.resubscribe();
            cmd_proc
                .last_once
                .store(true, std::sync::atomic::Ordering::SeqCst); // don't process the queue
            cmd_proc.process_command(super::Command::UpdateStatistics);
            drop(cmd_proc_locked);

            timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("should have received something by 1 second")
                .expect("should have received something");
        }
        .await;

        let app_state_lock = app_state.lock().await;
        let cmd_processor = app_state_lock.cmdprocessor.lock().await;

        assert_eq!(cmd_processor.as_ref().unwrap().queue_len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_queue_with_update_statistics() {
        let testing_appstate = new_testing_appstate().expect("can create test appstate");
        {
            // with pxls lock
            let mut pxclient = testing_appstate.pxlsclient.lock().await;
            mem::swap(
                &mut pxclient.user_ranks_ret_val,
                &mut Ok(vec![UserRank {
                    username: "vanorsigma".to_string(),
                    pixels: 123,
                }]),
            );
            mem::swap(
                &mut pxclient.user_profiles_ret_val,
                &mut Err(anyhow!("intentional mock error")),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        super::spawn_command_processor(app_state.clone(), MockWsHandler::default()).await;

        {
            let app_state_lock = app_state.lock().await;
            let cmd_processor_lock = app_state_lock.cmdprocessor.lock().await;
            let cmd_processor = cmd_processor_lock.as_ref().unwrap();

            let mut rx = cmd_processor.resubscribe();
            cmd_processor.queue_command(super::Command::UpdateStatistics);
            drop(cmd_processor_lock);
            drop(app_state_lock);

            timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("should have received something by 5 seconds")
                .expect("should have received something");
        }

        let app_state_lock = app_state.lock().await;
        let cmd_processor = app_state_lock.cmdprocessor.lock().await;

        assert_eq!(cmd_processor.as_ref().unwrap().queue_len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_faction_with_queue() {
        let testing_appstate = new_testing_appstate().expect("can create test appstate");
        {
            // with pxls lock
            let mut pxclient = testing_appstate.pxlsclient.lock().await;
            mem::swap(
                &mut pxclient.user_ranks_ret_val,
                &mut Ok(vec![UserRank {
                    username: "vanorsigma".to_string(),
                    pixels: 123,
                }]),
            );
            mem::swap(
                &mut pxclient.user_profiles_ret_val,
                &mut Ok(UserProfile {
                    discord_tag: Some("vanorsigma".to_string()),
                    faction_id: Some(69420),
                    pixels: Some(123),
                }),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        super::spawn_command_processor(app_state.clone(), MockWsHandler::default()).await;

        {
            let app_state_lock = app_state.lock().await;
            let database = app_state_lock.database.lock().await;
            let mut cmd_processor_lock = app_state_lock.cmdprocessor.lock().await;
            let cmd_processor = cmd_processor_lock.as_mut().unwrap();

            database
                .insert_user_record(UserRecord {
                    pxls_username: "vanorsigma".to_string(),
                    discord_tag: Some("vanorsigma".to_string()),
                    faction: Some(69420),
                    score: Some(123),
                    dirty_slate: Some(1),
                })
                .expect("can insert initial value into the database");
            cmd_processor.queue_command(super::Command::UpdateFaction("vanorsigma".to_string()));

            // forces the queue to terminate; this guarantees that the command has been processed
            let mut extracted_handle = tokio::task::spawn(async move {});
            std::mem::swap(&mut cmd_processor.queue_handle, &mut extracted_handle);
            cmd_processor
                .last_once
                .store(true, std::sync::atomic::Ordering::SeqCst);

            drop(database);
            drop(cmd_processor_lock);
            drop(app_state_lock);

            // i know this is scuffed, but delay for a second so that we can turn off the processor
            if !extracted_handle.is_finished() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = timeout(Duration::from_secs(5), extracted_handle)
                    .await
                    .expect("extracted handle timeouts after 5 seconds")
                    .expect("should be able to await from extracted handle");
            }
        }

        let app_state_lock = app_state.lock().await;
        let database = app_state_lock.database.lock().await;
        let record = database
            .get_user_record("vanorsigma")
            .expect("can get the record");

        assert_eq!(record.discord_tag, Some("vanorsigma".to_string()));
        assert_eq!(record.faction, Some(69420));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_from_websocket_with_queue() {
        let testing_appstate = new_testing_appstate().expect("can create test appstate");
        {
            // with pxls lock
            let mut pxclient = testing_appstate.pxlsclient.lock().await;
            mem::swap(
                &mut pxclient.user_profiles_ret_val,
                &mut Ok(UserProfile {
                    discord_tag: Some("vanorsigma".to_string()),
                    faction_id: Some(69420),
                    pixels: Some(123),
                }),
            );
            mem::swap(
                &mut pxclient.username_for_pixel_ret_val,
                &mut Ok(PxlsPixelResponse {
                    username: "vanorsigma".to_string(),
                    pixel_count: 123,
                }),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        let ws_handler = MockWsHandler::default();
        ws_handler
            .tx
            .send(PixelUpdate {
                pixels: vec![Pixel { x: 123, y: 456 }],
            })
            .expect("no error when sending a pixel update");
        super::spawn_command_processor(app_state.clone(), ws_handler).await;

        {
            let app_state_lock = app_state.lock().await;
            let database = app_state_lock.database.lock().await;
            let mut cmd_processor_lock = app_state_lock.cmdprocessor.lock().await;
            let cmd_processor = cmd_processor_lock.as_mut().unwrap();

            database
                .insert_user_record(UserRecord {
                    pxls_username: "vanorsigma".to_string(),
                    discord_tag: Some("vanorsigma".to_string()),
                    faction: Some(69420),
                    score: Some(123),
                    dirty_slate: Some(1),
                })
                .expect("can insert initial value into the database");

            // forces the queue to terminate; this guarantees that the command has been processed
            let mut extracted_handle = tokio::task::spawn(async move {});
            std::mem::swap(&mut cmd_processor.queue_handle, &mut extracted_handle);
            cmd_processor
                .last_once
                .store(true, std::sync::atomic::Ordering::SeqCst);

            drop(database);
            drop(cmd_processor_lock);
            drop(app_state_lock);

            // i know this is scuffed, but delay for a second so that we can turn off the processor
            if !extracted_handle.is_finished() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = timeout(Duration::from_secs(5), extracted_handle)
                    .await
                    .expect("extracted handle timeouts after 5 seconds")
                    .expect("should be able to await from extracted handle");
            }
        }

        let app_state_lock = app_state.lock().await;
        let database = app_state_lock.database.lock().await;
        let record = database
            .get_user_record("vanorsigma")
            .expect("can get the record");

        assert_eq!(record.discord_tag, Some("vanorsigma".to_string()));
        assert_eq!(record.faction, Some(69420));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_score_with_queue() {
        let testing_appstate = new_testing_appstate().expect("can create test appstate");
        {
            // with pxls lock
            let mut pxclient = testing_appstate.pxlsclient.lock().await;
            mem::swap(
                &mut pxclient.user_ranks_ret_val,
                &mut Ok(vec![UserRank {
                    username: "vanorsigma".to_string(),
                    pixels: 123,
                }]),
            );
            mem::swap(
                &mut pxclient.user_profiles_ret_val,
                &mut Ok(UserProfile {
                    discord_tag: Some("vanorsigma".to_string()),
                    faction_id: Some(69420),
                    pixels: Some(123),
                }),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        super::spawn_command_processor(app_state.clone(), MockWsHandler::default()).await;

        {
            let app_state_lock = app_state.lock().await;
            let database = app_state_lock.database.lock().await;
            let mut cmd_processor_lock = app_state_lock.cmdprocessor.lock().await;
            let cmd_processor = cmd_processor_lock.as_mut().unwrap();

            database
                .insert_user_record(UserRecord {
                    pxls_username: "vanorsigma".to_string(),
                    discord_tag: Some("vanorsigma".to_string()),
                    faction: Some(69420),
                    score: Some(123),
                    dirty_slate: Some(1),
                })
                .expect("can insert initial value into the database");
            cmd_processor.queue_command(super::Command::UpdateScore("vanorsigma".to_string()));

            // forces the queue to terminate; this guarantees that the command has been processed
            let mut extracted_handle = tokio::task::spawn(async move {});
            std::mem::swap(&mut cmd_processor.queue_handle, &mut extracted_handle);
            cmd_processor
                .last_once
                .store(true, std::sync::atomic::Ordering::SeqCst);

            drop(database);
            drop(cmd_processor_lock);
            drop(app_state_lock);

            // i know this is scuffed, but delay for a second so that we can turn off the processor
            if !extracted_handle.is_finished() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = timeout(Duration::from_secs(5), extracted_handle)
                    .await
                    .expect("extracted handle timeouts after 5 seconds")
                    .expect("should be able to await from extracted handle");
            }
        }

        let app_state_lock = app_state.lock().await;
        let database = app_state_lock.database.lock().await;
        let record = database
            .get_user_record("vanorsigma")
            .expect("can get the record");

        assert_eq!(record.discord_tag, Some("vanorsigma".to_string()));
        assert_eq!(record.score, Some(123));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_users_from_url() {
        env_logger::init();
        let mut testing_appstate = new_testing_appstate().expect("can create test appstate");
        {
            // prime ignore list with 420
            testing_appstate.ignore_factions_list.push(420);

            // with rentry lock
            let mut rentry = testing_appstate.rentry.lock().await;
            mem::swap(
                &mut rentry.get_users_from_url_response,
                &mut Ok(vec!["vanorsigma".to_string()]),
            );

            // with pxls lock
            let mut pxclient = testing_appstate.pxlsclient.lock().await;
            mem::swap(
                &mut pxclient.user_profiles_ret_val,
                &mut Ok(UserProfile {
                    discord_tag: Some("vanorsigma".to_string()),
                    faction_id: Some(69420),
                    pixels: Some(123),
                }),
            );
            mem::swap(
                &mut pxclient.username_for_pixel_ret_val,
                &mut Ok(PxlsPixelResponse {
                    username: "vanorsigma".to_string(),
                    pixel_count: 123,
                }),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        super::spawn_command_processor(app_state.clone(), MockWsHandler::default()).await;

        {
            let app_state_lock = app_state.lock().await;
            let mut cmd_processor_lock = app_state_lock.cmdprocessor.lock().await;
            let cmd_processor = cmd_processor_lock.as_mut().unwrap();

            cmd_processor.process_command(super::Command::UpdateUsersFromURL("does not matter".to_string(), 420));

            // forces the queue to terminate; this guarantees that the command has been processed
            let mut extracted_handle = tokio::task::spawn(async move {});
            std::mem::swap(&mut cmd_processor.queue_handle, &mut extracted_handle);
            cmd_processor
                .last_once
                .store(true, std::sync::atomic::Ordering::SeqCst);

            drop(cmd_processor_lock);
            drop(app_state_lock);

            // i know this is scuffed, but delay for a second so that we can turn off the processor
            if !extracted_handle.is_finished() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = timeout(Duration::from_secs(5), extracted_handle)
                    .await
                    .expect("extracted handle timeouts after 5 seconds")
                    .expect("should be able to await from extracted handle");
            }
        }

        let app_state_lock = app_state.lock().await;
        let database = app_state_lock.database.lock().await;
        let record = database
            .get_user_record("vanorsigma")
            .expect("can get the record");

        assert_eq!(record.faction, Some(420));
    }
}
