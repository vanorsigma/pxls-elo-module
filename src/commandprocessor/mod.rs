use core::time;
use std::{
    collections::VecDeque,
    sync::{atomic::AtomicBool, Arc},
};

mod types;

use crate::{
    appstate::AppState,
    database::{types::UserRecord, Database},
    pxlsclient::{PxlsClient, UserRank},
};
use anyhow::anyhow;
use tokio::{sync::Mutex, task::JoinHandle};
use types::UserRankResponse;

pub enum Command {
    UpdateStatistics,
    UpdateFaction(String),
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
    C: CommandProcessor<D, P>,
>
{
    /// This function will be called in a loop in a blocking task.
    /// Rate-limiting should be done by the implementation.
    /// If this function returns true, do_once will be called again. Else, the task will terminate
    fn do_once(&self) -> impl std::future::Future<Output = bool> + std::marker::Send;

    /// Creates Self, injecting the handle into the CommandProcessor
    /// It is the implementers responsibility to cause do_once to return false once the implementor is dropped
    fn inject_command_processor_into_appstate(
        app_state: Arc<Mutex<AppState<D, P, C>>>,
        handle: JoinHandle<()>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

pub struct CommandProcessorImpl<D: Database + Send + 'static, P: PxlsClient + Send + 'static> {
    app_state: Arc<Mutex<AppState<D, P, Self>>>,
    queue: Arc<Mutex<VecDeque<Command>>>,
    tx: tokio::sync::broadcast::Sender<CommandResponse>,
    rx: tokio::sync::broadcast::Receiver<CommandResponse>,
    #[allow(dead_code)]
    queue_handle: JoinHandle<()>,

    last_once: AtomicBool, // used to signal a drop
}

fn extract_result_vec_error<T, E>(vector: Vec<Result<T, E>>) -> Result<Vec<T>, E> {
    let mut result_without_error = vec![];
    for item in vector {
        result_without_error.push(item?);
    }
    Ok(result_without_error)
}

impl<D: Database + Send, P: PxlsClient + Send> CommandProcessorInternal<D, P, Self>
    for CommandProcessorImpl<D, P>
{
    async fn do_once(&self) -> bool {
        if self.last_once.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }

        if let Some(cmd) = self.queue.lock().await.pop_front() {
            self.process_command(cmd)
        }

        true
    }

    async fn inject_command_processor_into_appstate(
        app_state: Arc<Mutex<AppState<D, P, Self>>>,
        handle: JoinHandle<()>,
    ) {
        let (tx, rx) = tokio::sync::broadcast::channel(10);
        let ret_val = Self {
            app_state: app_state.clone(),
            tx,
            rx,
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

impl<D: Database + Send, P: PxlsClient + Send> Drop for CommandProcessorImpl<D, P> {
    fn drop(&mut self) {
        self.last_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // leave the handle alone to terminate
    }
}

impl<D: Database + Send, P: PxlsClient + Send> CommandProcessorImpl<D, P> {
    async fn update_statistics(
        app_state: Arc<Mutex<AppState<D, P, Self>>>,
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
                let record = database.get_user_record(&rank.username);
                let record_exists = record.is_ok();

                log::debug!("Record for {} exists? {}", rank.username, record_exists);

                match record {
                    Ok(mut r) => {
                        log::debug!("Inserting updated pixel counts");
                        r.pxls_username = rank.username.clone();
                        r.score = Some(rank.pixels);
                        database.insert_user_record(r)?;
                    }

                    Err(_) => database
                        .insert_user_record(UserRecord {
                            pxls_username: rank.username.clone(),
                            discord_tag: None,
                            faction: None,
                            score: Some(rank.pixels),
                            dirty_slate: Some(0),
                        })
                        .inspect(|_| {
                            log::debug!("Queueing the UpdateFaction command");
                            cmdprocessor.as_ref().inspect(|processor| {
                                processor
                                    .queue_command(Command::UpdateFaction(rank.username.clone()));
                            });
                        })?,
                };

                let new_record = database
                    .get_user_record(&rank.username)
                    .map_err(|e| anyhow!("cannot get new record, {}", e))?;

                Ok(UserRankResponse {
                    user: UserRank {
                        username: rank.username,
                        pixels: rank.pixels,
                    },
                    diff: if !record_exists {
                        rank.pixels
                    } else if rank.pixels > 0 {
                        rank.pixels - new_record.score.unwrap()
                    } else {
                        0
                    },
                })
            })
            .collect::<Vec<_>>();

        extract_result_vec_error(user_rank_responses)
    }

    async fn update_faction(
        app_state: Arc<Mutex<AppState<D, P, Self>>>,
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
        user.faction = profile.faction_id;
        user.discord_tag = profile.discord_tag;
        database.insert_user_record(user)?;

        Ok(())
    }
}

impl<D: Database + Send + 'static, P: PxlsClient + Send + 'static> CommandProcessorImpl<D, P> {
    fn process_command(&self, cmd: Command) {
        let app_state = self.app_state.clone();
        let tx = self.tx.clone();
        match cmd {
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
        }
    }
}

impl<D: Database + Send + 'static, P: PxlsClient + Send + 'static> CommandProcessor<D, P>
    for CommandProcessorImpl<D, P>
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
    C: CommandProcessor<D, P> + CommandProcessorInternal<D, P, C> + Send + 'static,
>(
    app_state: Arc<Mutex<AppState<D, P, C>>>,
) {
    let app_state_arc = app_state.clone();
    let handle = tokio::task::spawn(async move {
        let temp_app_state_lock = app_state_arc.lock().await;
        let cmdprocessor_lock = temp_app_state_lock.cmdprocessor.clone();
        drop(temp_app_state_lock);

        // to ensure that we release the cmdprocessor lock correctly
        let mut continue_processing = true;

        while continue_processing {
            let cmd_processor = cmdprocessor_lock.lock().await;
            match cmd_processor.as_ref() {
                Some(processor) => continue_processing = processor.do_once().await,
                None => continue, // cmd_processor hasn't settled in yet
            }
            // lock is released here
        }
    });

    C::inject_command_processor_into_appstate(app_state, handle).await;
}

#[cfg(test)]
mod tests {
    use std::{mem, sync::Arc, time::Duration};

    use anyhow::anyhow;
    use tokio::{sync::Mutex, time::timeout};

    use crate::{
        appstate::{new_memory_appstate, new_testing_appstate},
        commandprocessor::CommandProcessor,
        database::{types::UserRecord, Database},
        pxlsclient::{UserProfile, UserRank},
    };

    #[ignore = "this is a live test, requires actual credentials"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_real_update_statistics() {
        let app_state = Arc::new(Mutex::new(
            new_memory_appstate().expect("can create app state"),
        ));

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

    // TODO: real update faction probably

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
        super::spawn_command_processor(app_state.clone()).await;

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
        super::spawn_command_processor(app_state.clone()).await;

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
                }),
            );
        }

        let app_state = Arc::new(Mutex::new(testing_appstate));
        super::spawn_command_processor(app_state.clone()).await;

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
}
