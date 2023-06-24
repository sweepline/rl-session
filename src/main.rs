use anyhow::{anyhow, Result};
use boxcars::{HeaderProp, Replay};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use webhook::client::WebhookClient;

use clap::Parser;
use indoc::{formatdoc, indoc};
use std::{collections::HashMap, fs, path::PathBuf, str::FromStr};

/// A program for tracking scores while playing rocket league and publishing the running tally to discord.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Location to look for replays.
    #[arg(short, long)]
    location: Option<PathBuf>,
    /// The webhook API link from Discord channel integrations.
    #[arg(short, long)]
    webhook: Option<String>,
    /// Run without discord and print messages to stdout.
    #[arg(short, long)]
    no_discord: bool,
}

#[derive(Debug)]
struct Tally {
    player_stats: HashMap<String, PlayerStats>,
    games_played: usize,
}

#[derive(Debug)]
struct PlayerStats {
    times_seen: usize,
    wins: usize,
    losses: usize,
    score: (usize, usize),
    goals: (usize, usize),
    assists: (usize, usize),
    saves: (usize, usize),
    shots: (usize, usize),
}

const BOT_NAME: &str = "Rocket League Session";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.webhook.is_none() && !args.no_discord {
        return Err(anyhow!(
            "You must either provide a webhook with --webhook or run with --no-discord"
        ));
    }

    let client: WebhookClient = WebhookClient::new(&args.webhook.unwrap_or_default());

    let Some(location) = args.location.or_else(|| PathBuf::from_str(&format!(r"C:\Users\{}\AppData\Roaming\bakkesmod\bakkesmod\data\replays", whoami::username())).ok()) else {
        return Err(anyhow!("Location was not valid and default location did not work. Please supply a path to the replay folder"));
    };
    println!("Looking for saves in: {}", location.to_string_lossy());

    let (tx, rx) = std::sync::mpsc::channel();
    // This example is a little bit misleading as you can just create one Config and use it for all watchers.
    // That way the pollwatcher specific stuff is still configured, if it should be used.
    let mut watcher: Box<dyn Watcher> =
        Box::new(RecommendedWatcher::new(tx, Config::default()).unwrap());

    // watch some stuff
    let Ok(_) = watcher
        .watch(&location, RecursiveMode::NonRecursive) else {
            return Err(anyhow!("Location was not valid and default location did not work. Please supply a path to the replay folder"));
        };

    // Set up the running tally.
    let mut tally: Tally = Tally {
        player_stats: HashMap::new(),
        games_played: 0,
    };

    if !args.no_discord {
        let _res = client.send(|message| {
            message.username(BOT_NAME).embed(|embed| {
                embed
                    .title("Starting new session")
                    .description(indoc! {
                        "The bot will try to single out the people that plays multiple times in the session, on either team.
                        Please make sure to install Bakkesmod and make _Auto replay uploader_ do export to the filepath specified by you or the program.
                        Stats are in the form: accumulated (last game)
                    "})
            })
        }).await;
    }

    let mut current_file: Option<PathBuf> = None;
    for e in rx {
        match e {
            Ok(event) => {
                let Event {
                    kind,
                    paths,
                    attrs: _,
                } = event;

                if let Some(p) = paths.get(0) {
                    let file_name = p.file_name().unwrap_or_default().to_string_lossy();

                    // Bakkesmod opens the file (Create) then writes it (Modify).
                    match kind {
                        EventKind::Create(_e) => {
                            println!("Replay created: {}", file_name);
                            println!("Waiting for write");
                            current_file = Some(p.clone());
                            continue;
                        }
                        EventKind::Modify(_e) => {
                            if let Some(c) = &current_file {
                                if c == p {
                                    println!("Replay written: {}", file_name);
                                    println!("Sending stats");
                                    current_file = None;
                                } else {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                        }
                        _ => {
                            continue;
                        }
                    }

                    let extension = p.extension();
                    if extension.is_none() || !extension.is_some_and(|os| os == "replay") {
                        continue;
                    }
                    let Ok(replay) = parse_rl(p) else {
                        continue;
                    };
                    let Some(stats) = replay.properties.iter().find(|(s, _)| s == "PlayerStats") else {
                        eprintln!("No playerstats for replay");
                        continue;
                    };
                    let (_, stats) = stats;
                    let HeaderProp::Array(stats) = stats else {
                        continue;
                    };

                    let team0_score = replay
                        .properties
                        .iter()
                        .find(|(s, _)| s == "Team0Score")
                        .map(|(_, v)| v.as_i32().unwrap_or_default())
                        .unwrap_or_default();
                    let team1_score = replay
                        .properties
                        .iter()
                        .find(|(s, _)| s == "Team1Score")
                        .map(|(_, v)| v.as_i32().unwrap_or_default())
                        .unwrap_or_default();
                    let team_win_lose = if team0_score == team1_score {
                        (2, 2)
                    } else if team0_score > team1_score {
                        (0, 1)
                    } else {
                        (1, 0)
                    };

                    // Accumulate stats
                    for player_stat in stats {
                        let mut name: Option<String> = None;
                        let mut score: usize = 0;
                        let mut goals: usize = 0;
                        let mut assists: usize = 0;
                        let mut saves: usize = 0;
                        let mut shots: usize = 0;
                        let mut team: usize = 0;
                        for (key, prop) in player_stat {
                            match (key.as_str(), prop) {
                                ("Name", HeaderProp::Str(v)) => name = Some(v.to_string()),
                                ("Score", HeaderProp::Int(v)) => score = *v as usize,
                                ("Goals", HeaderProp::Int(v)) => goals = *v as usize,
                                ("Assists", HeaderProp::Int(v)) => assists = *v as usize,
                                ("Saves", HeaderProp::Int(v)) => saves = *v as usize,
                                ("Shots", HeaderProp::Int(v)) => shots = *v as usize,
                                ("Team", HeaderProp::Int(v)) => team = *v as usize,
                                _ => {}
                            }
                        }

                        let did_win = team == team_win_lose.0 as usize;
                        let did_lose = team == team_win_lose.1;

                        if let Some(name) = name {
                            let stats = tally.player_stats.entry(name);
                            stats
                                .and_modify(|stats| {
                                    stats.times_seen += 1;
                                    stats.wins += did_win as usize;
                                    stats.losses += did_lose as usize;
                                    stats.score = (stats.score.0 + score, score);
                                    stats.goals = (stats.goals.0 + goals, goals);
                                    stats.assists = (stats.assists.0 + assists, assists);
                                    stats.saves = (stats.saves.0 + saves, saves);
                                    stats.shots = (stats.shots.0 + shots, shots);
                                })
                                .or_insert(PlayerStats {
                                    times_seen: 1,
                                    score: (score, score),
                                    goals: (goals, goals),
                                    assists: (assists, assists),
                                    saves: (saves, saves),
                                    shots: (shots, shots),
                                    wins: did_win as usize,
                                    losses: did_lose as usize,
                                });
                        }
                    }
                    tally.games_played += 1;

                    // Write to discord.
                    let mut stat_message =
                        format!("## Game {games} finished\n\n", games = tally.games_played);
                    let mut sorted: Vec<(&String, &PlayerStats)> =
                        tally.player_stats.iter().collect();
                    sorted.sort_unstable_by(|a, b| b.1.score.cmp(&a.1.score));
                    for (name, stats) in sorted {
                        if stats.times_seen != tally.games_played
                            && stats.times_seen <= usize::max(3, tally.games_played / 2)
                        {
                            // This should sufficiently remove people not playing with you.
                            continue;
                        }
                        let PlayerStats {
                            times_seen,
                            score,
                            goals,
                            assists,
                            saves,
                            shots,
                            wins,
                            losses,
                        } = stats;
                        let player_msg = formatdoc! {"
                            ### {name}
                            *Played {times_seen} games*
                            - Wins/Losses: {wins}/{losses}
                            - Score: {score_tally} ({score})
                            - Goals: {goals_tally} ({goals})
                            - Assists: {assists_tally} ({assists})
                            - Saves: {saves_tally} ({saves})
                            - Shots: {shots_tally} ({shots})
                        ",
                        name=name,
                        times_seen=times_seen,
                        wins=wins,
                        losses=losses,
                        score_tally=score.0,
                        score=score.1,
                        goals_tally=goals.0,
                        goals=goals.1,
                        assists_tally=assists.0,
                        assists=assists.1,
                        saves_tally=saves.0,
                        saves=saves.1,
                        shots_tally=shots.0,
                        shots=shots.1
                        };
                        stat_message.push_str(&player_msg);
                        // stat_message.push_str("\n");
                    }

                    if !args.no_discord {
                        let res = client
                            .send(|message| {
                                message
                                    .username(BOT_NAME)
                                    .embed(|embed| embed.description(&stat_message))
                            })
                            .await;
                        if res.is_err() {
                            eprintln!("Failed to send message to discord webhook");
                            continue;
                        };
                        eprintln!("Sent stats to discord\n");
                    } else {
                        print!("{}", stat_message);
                    }
                }
            }
            Err(e) => {
                eprintln!("{:?}", e);
            }
        }
    }

    Ok(())
}

fn parse_rl(filename: &PathBuf) -> Result<Replay> {
    let data = fs::read(filename)?;
    let replay = boxcars::ParserBuilder::new(&data)
        .never_parse_network_data()
        .parse()?;
    Ok(replay)
}
