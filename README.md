# pxls Elo Module

> Note: I have approval from pxls.space admins to automate enough aspects to perform the task you see below.

## Approach

This is a module that should eventually end up in `neuro-chat-elo`, possibly only after `live-elo`.

General "how it works":
- (Timer 1) Every hour or so, the module will pull leaderboard from Pxls. In particular, it'll pull the users leaderboard for the current canvas.
- It will look at a database (sqlite) to check if an entry with the name already exists.
- If the name exists: we store the score, and emit the difference with the old score (this is the library output)
- If the name does not exist, we visit their user profile to pull their public discord username and public shown faction(s?), then emit the initial score.
- In both cases: We mark the user as "dirty" in the database
- (Timer 2) Every 24 hours, the module will go through ALL users in the database one by one. Once it has figured out the new faction of the user, it will mark the user as "clean" in the database. This can run concurrently with the Timer 1; it is intentional to have weak consistency during the updating period.
- (Timer 1 & 2) As a rate-limiting measure, there will only be one requests to get the faction per user. Hence, this will be a long-running task (for 1000 users, this will take ~17 minutes)

The following are the overall decisions:
- Database will be a simple SQLite. There are no plans to scale the elo module to be larger.
    - Pxls Username
    - Discord Tag
    - Score
    - Dirty State
- No backups will be designed. SQLite is more or less a cache.
- To support rate-limiting, each request will be implemented as a "Command"
    - A "pull leaderboard" command will pull the data, query the db, and create any necessary "get faction" command
    - The command processor will rate-limit at command-level; if it sees "get faction", it will look for another command to run
    - By design, the command processor will be sequential. Since, as far as the server is concerned, this module is a single client.
