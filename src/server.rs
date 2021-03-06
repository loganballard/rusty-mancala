use crate::client_input_handler::{client_initiate_disconnect, leave_game};
use crate::game_objects::*;
use crate::proto::*;
use crate::server_input_handler::*;

use crate::constants::SUPER_SECRET_PASSWORD;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

pub type MsgChanSender = mpsc::Sender<(u32, Msg)>;
pub type MsgChanReceiver = mpsc::Receiver<(u32, Msg)>;

/// Handle client message
/// Messages from client are deserialized, checked for errors and
/// passed back to data management.
#[cfg_attr(tarpaulin, skip)]
fn handle_client_input_msg(buffer: &[u8; 512], size: usize) -> Msg {
    let client_msg: Msg = bincode::deserialize(&buffer[0..size]).unwrap();
    debug!("TCP data received: {:?}", client_msg);
    if client_msg.status != Status::Ok {
        // TODO - some sort of error checking
    }
    client_msg
}

/// When a client terminates the connection, clear them from the shared data structures
/// as well as booting them from a game if they're in one currently
#[cfg_attr(tarpaulin, skip)]
fn handle_client_disconnect(
    snd_channel: &Arc<Mutex<MsgChanSender>>,
    rec_channel: &MsgChanReceiver,
    user_id: u32,
    in_game: bool,
) {
    info!("Client terminated connection");
    let mut boot_msgs: Vec<Msg> = Vec::new();
    if in_game {
        boot_msgs.push(leave_game());
    }
    boot_msgs.push(client_initiate_disconnect());
    for msg in boot_msgs {
        snd_channel.lock().unwrap().send((user_id, msg)).unwrap();
        rec_channel.recv().expect("something wrong");
    }
}

#[cfg_attr(tarpaulin, skip)]
fn shutdown_stream(stream: &TcpStream) {
    let res = stream.shutdown(Shutdown::Both);
    match res {
        Ok(_) => {}
        Err(e) => {
            error!("Stream had to be force shutdowned: {}", e);
        }
    }
}

/// Per-client tcp connection handler
/// TCP input is received from client connection and message is handled.
/// Messages are processed by handle_client_input_msg and appropriate actions
/// are taken.  Responses sent to client over TCP.
#[cfg_attr(tarpaulin, skip)]
fn handle_each_client_tcp_connection(
    mut stream: TcpStream,
    snd_channel: &Arc<Mutex<MsgChanSender>>,
    rec_channel: &MsgChanReceiver,
    user_id: u32,
) {
    let mut buffer = [0; 512];
    let mut in_game: bool = false;
    if !is_client_authorized(&mut stream) {
        handle_client_disconnect(&snd_channel, rec_channel, user_id, in_game);
        shutdown_stream(&stream);
        return; // not authorized, boot this client
    }
    loop {
        match stream.read(&mut buffer) {
            Ok(size) => {
                if size == 0 {
                    error!("client {} disconnected unexpectedly!", user_id);
                    handle_client_disconnect(&snd_channel, rec_channel, user_id, in_game);
                    shutdown_stream(&stream);
                    break;
                }
                let msg_to_send_to_manager: Msg = handle_client_input_msg(&buffer, size);
                snd_channel
                    .lock()
                    .unwrap()
                    .send((user_id, msg_to_send_to_manager))
                    .unwrap();
                let response_from_manager: (u32, Msg) =
                    rec_channel.recv().expect("something wrong");
                if response_from_manager.1.game_status == GameStatus::InGame && !in_game {
                    in_game = true;
                }
                response_from_manager.1.serialize(&mut buffer);
                stream.write_all(&buffer).unwrap();
                stream.flush().unwrap();
                debug!("TCP response sent: {:?}", &response_from_manager.1);
                if response_from_manager.1.command == Commands::KillClient {
                    info!("client {} killed!", user_id);
                    shutdown_stream(&stream);
                    break;
                }
            }
            Err(e) => {
                error!("stream object is gone, client most likely disconnected");
                println!("error: {}", e);
                shutdown_stream(&stream);
            }
        }
    }
}

/// Data Management
/// Thread spun from master process that allows for sharing of data.
/// Game states are recorded as well as per-connection client information.
/// Messages are received from the TCP connection manager and handled
/// based on message content.  Responses are sent to TCP Connection Manager
#[cfg_attr(tarpaulin, skip)]
fn data_manager(
    cli_comms: Arc<Mutex<HashMap<u32, MsgChanSender>>>,
    rec_server_master: MsgChanReceiver,
    game_list_mutex: Arc<Mutex<Vec<GameState>>>,
    id_game_map_mutex: Arc<Mutex<HashMap<u32, u32>>>,
    active_nicks_mutex: Arc<Mutex<HashSet<String>>>,
    id_nick_map_mutex: Arc<Mutex<HashMap<u32, String>>>,
) {
    loop {
        let rec: (u32, Msg) = rec_server_master
            .recv()
            .expect("didn't get a message or something");
        let cli_com_base = cli_comms.lock().unwrap();
        let res_comm_channel = cli_com_base.get(&rec.0).expect("no id match");
        let status: GameStatus = rec.1.game_status.clone();
        let cmd: Commands = rec.1.command.clone();
        if status == GameStatus::NotInGame {
            let server_res: Msg = handle_out_of_game(
                cmd,
                &game_list_mutex,
                &id_game_map_mutex,
                &active_nicks_mutex,
                &id_nick_map_mutex,
                &rec.1,
                rec.0,
            );
            res_comm_channel
                .send((rec.0, server_res))
                .expect("Error sending to thread");
            continue;
        }
        let server_res: Msg =
            handle_in_game(cmd, &game_list_mutex, &id_game_map_mutex, &rec.1, rec.0);
        res_comm_channel
            .send((rec.0, server_res))
            .expect("Error sending to thread");
    }
}

/// check the authorization of client
///
#[cfg_attr(tarpaulin, skip)]
fn is_client_authorized(stream: &mut TcpStream) -> bool {
    let mut buffer = [0; 512];
    match stream.read(&mut buffer) {
        Ok(size) => {
            if &buffer[0..size] == SUPER_SECRET_PASSWORD.as_bytes() {
                info!("Client authenticated, granting access");
                stream.write_all(b"nice").unwrap();
                stream.flush().unwrap();
                return true;
            }
            error!("Client supplied the wrong password");
            false
        }
        Err(e) => {
            error!("User not authorized! Terminating. error: {}", e);
            false
        }
    }
}

/// Set up new client
/// Gives initial values to client as well as opening a new communication
/// channel to the data manager.
#[cfg_attr(tarpaulin, skip)]
fn set_up_new_client_tcp_connection(
    client_comms_mutex: &Arc<Mutex<HashMap<u32, MsgChanSender>>>,
    client_to_server_sender: &Arc<Mutex<MsgChanSender>>,
    cur_id: u32,
    active_nicks_mutex: &Arc<Mutex<HashSet<String>>>,
    id_nick_map_mutex: &Arc<Mutex<HashMap<u32, String>>>,
) -> (Arc<Mutex<MsgChanSender>>, MsgChanReceiver) {
    let (send_server, rec_channel): (MsgChanSender, MsgChanReceiver) = mpsc::channel();
    client_comms_mutex
        .lock()
        .unwrap()
        .insert(cur_id, send_server);
    let initial_nick: String = "user_".to_string() + &cur_id.to_string();
    active_nicks_mutex
        .lock()
        .unwrap()
        .insert(initial_nick.clone());
    id_nick_map_mutex
        .lock()
        .unwrap()
        .insert(cur_id, initial_nick);
    let snd_channel = Arc::clone(&client_to_server_sender);
    (snd_channel, rec_channel)
}

/// TCP Connection Manager
/// Thread spawned from the master process.  Will handle each client that
/// connects. Performs some initialization after connection and then loops
/// on handling client input.
#[cfg_attr(tarpaulin, skip)]
fn tcp_connection_manager(
    port_int: u32,
    client_comms_mutex: Arc<Mutex<HashMap<u32, MsgChanSender>>>,
    client_to_server_sender: Arc<Mutex<MsgChanSender>>,
    active_nicks_mutex: Arc<Mutex<HashSet<String>>>,
    id_nick_map_mutex: Arc<Mutex<HashMap<u32, String>>>,
) {
    let connection = format!("0.0.0.0:{}", port_int);
    let listener = TcpListener::bind(&connection).unwrap();
    let mut cur_id: u32 = 1;

    info!("Server listening on {}", connection);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                info!("New connection: {}", stream.peer_addr().unwrap());
                let channels: (Arc<Mutex<MsgChanSender>>, MsgChanReceiver) =
                    set_up_new_client_tcp_connection(
                        &client_comms_mutex,
                        &client_to_server_sender,
                        cur_id,
                        &active_nicks_mutex,
                        &id_nick_map_mutex,
                    );
                let t = thread::Builder::new()
                    .name(format!("thread: {}", cur_id))
                    .spawn(move || {
                        handle_each_client_tcp_connection(stream, &channels.0, &channels.1, cur_id);
                    });
                match t {
                    Ok(_) => {}
                    Err(_) => {
                        error!("An error occurred, terminating connection");
                    }
                }
                cur_id += 1;
            }
            Err(e) => {
                error!("Error: {}", e);
            }
        }
    }
}

/// The main entry point into the server side of the Mancala program.  Does all
/// setup for managing data as well as TCP connections.  Creates shared data structures
/// accessible to all running threads.  Spawns two main management functions:
///    - Data Manager
///      Handles all the shared data structures.  Maintains game state and communicates
///      with spawned per-client threads
///    - TCP Connection Manager
///      Spawns a new thread per client TCP connection and manages IO with the client
///      communicates via channels with data manager
#[cfg_attr(tarpaulin, skip)]
pub fn run_server(port_int: u32) {
    let game_list: Vec<GameState> = vec![];
    let game_list_mutex = Arc::new(Mutex::new(game_list));

    let id_to_game_map: HashMap<u32, u32> = HashMap::new();
    let id_to_game_map_mutex = Arc::new(Mutex::new(id_to_game_map));

    let active_nicks: HashSet<String> = HashSet::new();
    let active_nicks_mutex = Arc::new(Mutex::new(active_nicks));

    let id_nick_map: HashMap<u32, String> = HashMap::new();
    let id_nick_map_mutex = Arc::new(Mutex::new(id_nick_map));

    let (send_client_master, rec_server_master): (MsgChanSender, MsgChanReceiver) = mpsc::channel();
    let client_to_server_sender = Arc::new(Mutex::new(send_client_master));

    let client_comms: HashMap<u32, MsgChanSender> = HashMap::new();
    let client_comms_mutex = Arc::new(Mutex::new(client_comms));

    let client_comms_mutex_tcp_manager_copy = Arc::clone(&client_comms_mutex);
    let client_comms_mutex_client_manager_copy = Arc::clone(&client_comms_mutex);
    let active_nicks_mutex_data_copy = Arc::clone(&active_nicks_mutex);
    let id_nick_map_mutex_data_copy = Arc::clone(&id_nick_map_mutex);
    let active_nicks_mutex_tcp_copy = Arc::clone(&active_nicks_mutex);
    let id_nick_map_mutex_tcp_copy = Arc::clone(&id_nick_map_mutex);
    let id_game_map_mutex_copy = Arc::clone(&id_to_game_map_mutex);

    thread::spawn(move || {
        data_manager(
            client_comms_mutex_client_manager_copy,
            rec_server_master,
            game_list_mutex,
            id_game_map_mutex_copy,
            active_nicks_mutex_data_copy,
            id_nick_map_mutex_data_copy,
        );
    });
    tcp_connection_manager(
        port_int,
        client_comms_mutex_tcp_manager_copy,
        client_to_server_sender,
        active_nicks_mutex_tcp_copy,
        id_nick_map_mutex_tcp_copy,
    );
}
