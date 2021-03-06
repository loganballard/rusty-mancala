use crate::game_objects::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum Headers {
    Read,
    Write,
    Response,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum Status {
    Ok,
    NotOk,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum GameStatus {
    InGame,
    NotInGame,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum Commands {
    InitSetup,
    SetNick,
    ListGames,
    ListUsers,
    MakeNewGame,
    JoinGame,
    LeaveGame,
    GetCurrentGamestate,
    MakeMove,
    GameIsOver,
    KillMe,
    KillClient,
    Reply,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Msg {
    pub status: Status,
    pub headers: Headers,
    pub command: Commands,
    pub game_status: GameStatus,
    pub data: String,
    pub game_state: GameState,
}

impl Msg {
    pub fn serialize(&self, buf: &mut [u8; 512]) {
        let encoded: Vec<u8> = bincode::serialize(&self).unwrap();
        buf[..encoded.len()].clone_from_slice(&encoded[..]);
    }
}

#[test]
fn test_serialize_msg() {
    let msg1: Msg = Msg {
        status: Status::Ok,
        headers: Headers::Write,
        command: Commands::SetNick,
        game_status: GameStatus::NotInGame,
        data: "data".to_string(),
        game_state: GameState::new_empty(),
    };
    let mut buf: [u8; 512] = [0; 512];
    msg1.serialize(&mut buf);
    let msg2: Msg = bincode::deserialize(&buf[..]).unwrap();
    assert_eq!(msg1, msg2);
}
