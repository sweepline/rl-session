# RL-Session

A program for tracking scores while playing rocket league and publishing the running tally to discord.

Right now it does not persist any data, which means sessions are the duration the program is kept open.

To use this, you have to download it from the Releases tab and get a webhook API key from channel settings in discord.

You can then run it from the terminal like so:
```
.\rl-session.exe -w https://discord.com/api/webhooks/{NUMBERS}/{SOME_LONG_STRING}
```

The program can also be run with `--no-discord` to just output the results to stdout.