/// Copyright 2023 Alexander Kalashnikov

///    Licensed under the Apache License, Version 2.0 (the "License");
///    you may not use this file except in compliance with the License.
///    You may obtain a copy of the License at

///        http://www.apache.org/licenses/LICENSE-2.0

///    Unless required by applicable law or agreed to in writing, software
///    distributed under the License is distributed on an "AS IS" BASIS,
///    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
///    See the License for the specific language governing permissions and
///    limitations under the License.
use std::{io::Read, sync::mpsc};

use uni_lib_client_rs::uni;

static AUDIO_STREAM_VTABLE: uni::mpf_audio_stream_vtable_t = uni::mpf_audio_stream_vtable_t {
    destroy: Some(recog_app_stream_destroy),
    open_rx: Some(recog_app_stream_open),
    close_rx: Some(recog_app_stream_close),
    read_frame: Some(recog_app_stream_read),
    open_tx: None,
    close_tx: None,
    write_frame: None,
    trace: None,
};

struct App {
    sender: mpsc::Sender<()>,
}

struct RecogChannel {
    streaming: bool,
    path: String,
    source: Option<std::fs::File>,
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        println!("ERROR: expect only one argument, path to audio file.");
        return;
    }
    let pool: *mut uni::apr_pool_t = unsafe { uni::apt_pool_create() };
    println!("On pool {:?}", pool);
    let dir_layout =
        unsafe { uni::apt_default_dir_layout_create(b"/opt/unimrcp\0".as_ptr() as _, pool) };
    let client = unsafe { uni::unimrcp_client_create(dir_layout) };
    if client.is_null() {
        println!("ERROR: could not create client instance.");
        return;
    }
    let (tx, rx) = mpsc::channel();
    let recog_app = Box::leak(Box::new(App { sender: tx })) as *mut App;
    let app = unsafe { uni::mrcp_application_create(Some(message_handler), recog_app as _, pool) };
    let registered =
        unsafe { uni::mrcp_client_application_register(client, app, b"recog\0".as_ptr() as _) };
    println!("Application {:?} registered = {}", app, registered);

    unsafe {
        uni::mrcp_client_start(client);
    }

    let path = args.last().unwrap().to_owned();
    unsafe {
        application_start(app, path);
    }

    println!("Received: {:?}", rx.recv());

    unsafe {
        uni::mrcp_client_shutdown(client);
        uni::mrcp_client_destroy(client);
        let _ = Box::from_raw(recog_app);
    }
}

unsafe fn application_start(app: *mut uni::mrcp_application_t, path: String) {
    let session =
        uni::mrcp_application_session_create(app, b"uni2\0".as_ptr() as _, std::ptr::null_mut());
    if session.is_null() {
        println!("ERROR: could not create session.");
        return;
    }
    let channel = recog_application_channel_create(session, path);
    if channel.is_null() {
        println!("ERROR: could not create channel.");
        uni::mrcp_application_session_destroy(session);
        return;
    }
    let channel_added = uni::mrcp_application_channel_add(session, channel);
    println!("Channel added = {}", channel_added);
    if channel_added != uni::TRUE {
        println!("ERROR: could not add channel to session.");
        uni::mrcp_application_session_destroy(session);
    }
}

unsafe fn recog_application_channel_create(
    session: *mut uni::mrcp_session_t,
    path: String,
) -> *mut uni::mrcp_channel_t {
    let session_pool = uni::mrcp_application_session_pool_get(session);

    println!("Create channel on session pool {:?}", session_pool);
    let recog_channel = Box::leak(Box::new(RecogChannel {
        streaming: false,
        path,
        source: None,
    })) as *mut RecogChannel;

    println!("Create capabilities on pool {:?}", session_pool);
    let capabilities =
        uni::mpf_stream_capabilities_create(uni::STREAM_DIRECTION_RECEIVE, session_pool);
    uni_lib_client_rs::inline_mpf_codec_capabilities_add(
        &mut (*capabilities).codecs as _,
        uni::MPF_SAMPLE_RATE_8000 as _,
        b"LPCM\0".as_ptr() as _,
    );

    println!("Create termination.");
    let termination = uni::mrcp_application_audio_termination_create(
        session,
        &AUDIO_STREAM_VTABLE as _,
        capabilities,
        recog_channel as _,
    );

    println!(
        "Creating channel with capabilities {:?} and termination {:?}",
        capabilities, termination
    );
    let channel = uni::mrcp_application_channel_create(
        session,
        uni::MRCP_RECOGNIZER_RESOURCE as _,
        termination,
        std::ptr::null_mut(),
        recog_channel as _,
    );
    println!("Created channel: {:?}", channel);

    channel
}

unsafe extern "C" fn message_handler(msg: *const uni::mrcp_app_message_t) -> uni::apt_bool_t {
    let recog_app = uni::mrcp_application_object_get((*msg).application) as *mut App;

    println!("Application message handler. Handle: {:?}", *msg);
    let msg_type = (*msg).message_type;
    match msg_type {
        uni::MRCP_APP_MESSAGE_TYPE_SIGNALING => {
            let signal = &(*msg).sig_message;
            let status = signal.status;
            match signal.message_type {
                uni::MRCP_SIG_MESSAGE_TYPE_RESPONSE => {
                    let cmd = signal.command_id;
                    println!("Response comes. {}", cmd);
                    match cmd {
                        uni::MRCP_SIG_COMMAND_SESSION_UPDATE => {
                            println!("Session updated. Status = {}", status);
                        }
                        uni::MRCP_SIG_COMMAND_SESSION_TERMINATE => unsafe {
                            println!("Session terminate signal!");
                            uni::mrcp_application_session_destroy((*msg).session);
                            (*recog_app).sender.send(()).unwrap();
                        },
                        uni::MRCP_SIG_COMMAND_CHANNEL_ADD => {
                            process_channel_added((*msg).session, (*msg).channel, status);
                        }
                        uni::MRCP_SIG_COMMAND_CHANNEL_REMOVE => {
                            process_channel_remove((*msg).session, (*msg).channel)
                        }
                        uni::MRCP_SIG_COMMAND_RESOURCE_DISCOVER => {
                            println!("Client cannot discover resource.");
                        }
                        _ => println!("Unexpected command."),
                    }
                }
                uni::MRCP_SIG_MESSAGE_TYPE_EVENT => {
                    let event = signal.event_id;
                    println!("Event comes. {}", event);
                    if event == uni::MRCP_SIG_EVENT_TERMINATE {
                        println!("Do nothing with it.");
                    } else {
                        println!("Unexpected event.");
                    }
                }
                uni::MRCP_SIG_MESSAGE_TYPE_REQUEST => println!("Client cannot handle requests."),
                _ => println!("Unexpected signal"),
            }
        }
        uni::MRCP_APP_MESSAGE_TYPE_CONTROL => {
            process_control_message((*msg).session, (*msg).channel, (*msg).control_message)
        }
        _ => println!("Unexpected message"),
    }
    uni::TRUE
}

unsafe extern "C" fn recog_app_stream_destroy(
    stream: *mut uni::mpf_audio_stream_t,
) -> uni::apt_bool_t {
    println!("Destroy stream: {:?}", stream);
    uni::TRUE
}
unsafe extern "C" fn recog_app_stream_open(
    stream: *mut uni::mpf_audio_stream_t,
    codec: *mut uni::mpf_codec_t,
) -> uni::apt_bool_t {
    println!("Open stream {:?} with codec {:?}", stream, codec);
    uni::TRUE
}
unsafe extern "C" fn recog_app_stream_close(
    stream: *mut uni::mpf_audio_stream_t,
) -> uni::apt_bool_t {
    println!("Close stream {:?}", stream);
    uni::TRUE
}
unsafe extern "C" fn recog_app_stream_read(
    stream: *mut uni::mpf_audio_stream_t,
    frame: *mut uni::mpf_frame_t,
) -> uni::apt_bool_t {
    let recog_channel = (*stream).obj as *mut RecogChannel;
    if !recog_channel.is_null() && (*recog_channel).streaming {
        let fr = std::slice::from_raw_parts_mut(
            (*frame).codec_frame.buffer as *mut u8,
            (*frame).codec_frame.size,
        );
        (*frame).type_ |= uni::MEDIA_FRAME_TYPE_AUDIO as i32;
        if let Some(source) = &mut (*recog_channel).source {
            let n = source.read(fr).unwrap_or(0);
            fr[n..].fill(0);
            if n == 0 {
                (*recog_channel).source = None;
            }
        } else {
            fr.fill(0);
        }
    }
    uni::TRUE
}

fn process_control_message(
    session: *mut uni::mrcp_session_t,
    channel: *mut uni::mrcp_channel_t,
    message: *mut uni::mrcp_message_t,
) {
    unsafe {
        let recog_channel = uni::mrcp_application_channel_object_get(channel) as *mut RecogChannel;
        let msg_type = (*message).start_line.message_type;
        println!("Control message: {:?}", *message);

        match msg_type {
            uni::MRCP_MESSAGE_TYPE_RESPONSE => {
                if (*message).start_line.method_id == uni::RECOGNIZER_RECOGNIZE as _ {
                    if (*message).start_line.request_id == uni::MRCP_REQUEST_STATE_INPROGRESS {
                        if !recog_channel.is_null() {
                            (*recog_channel).streaming = true;
                        }
                    } else {
                        println!(
                            "Server did not start to recognize. Tear down the channel {:?}",
                            *channel
                        );
                        uni::mrcp_application_channel_remove(session, channel);
                    }
                } else {
                    println!(
                        "Unexpected response method id {}",
                        (*message).start_line.method_id
                    );
                }
            }
            uni::MRCP_MESSAGE_TYPE_EVENT => match (*message).start_line.method_id as u32 {
                uni::RECOGNIZER_START_OF_INPUT => {
                    println!("Server received voice data AKA start of input.")
                }
                uni::RECOGNIZER_RECOGNITION_COMPLETE => {
                    if uni_lib_client_rs::inline_mrcp_resource_header_property_check(
                        message,
                        uni::RECOGNIZER_HEADER_COMPLETION_CAUSE as _,
                    ) == uni::TRUE
                    {
                        let resource_header =
                            uni_lib_client_rs::inline_mrcp_resource_header_get(message)
                                as *mut uni::mrcp_recog_header_t;
                        let completion_cause = (*resource_header).completion_cause;
                        println!(
                            "Completion-Cause: {:03} -- {:?}",
                            completion_cause,
                            (*resource_header).completion_reason
                        );
                        match completion_cause {
                            0 => {
                                let result = std::slice::from_raw_parts(
                                    (*message).body.buf as *const u8,
                                    (*message).body.length,
                                );
                                let text = std::str::from_utf8(result)
                                    .unwrap_or("Сервер прислал результат не в кодировке UTF-8.");
                                println!("RECOGNITION COMPLETE successfully,\n{:?}", text);
                            }
                            2 => {
                                println!("NOINPUT detected by server.")
                            }
                            _ => {}
                        }
                    } else {
                        println!("FATAL ERROR: no completion cause from the server.")
                    }
                    if !recog_channel.is_null() {
                        (*recog_channel).streaming = false;
                    }
                    uni::mrcp_application_channel_remove(session, channel);
                }
                _ => println!("Unexpected event."),
            },
            _ => println!("Unexpected control message type."),
        }
    }
}

fn process_channel_added(
    session: *mut uni::mrcp_session_t,
    channel: *mut uni::mrcp_channel_t,
    status: u32,
) {
    println!("Added channel. Status = {}", status);
    if status == uni::MRCP_SIG_STATUS_CODE_SUCCESS {
        let recog_msg = create_recognize_message(session, channel);
        if !recog_msg.is_null() {
            unsafe {
                uni::mrcp_application_message_send(session, channel, recog_msg);
            }
            prepare_channel(channel);
        } else {
            println!("Could not create RECOGNIZE message.");
        }
    } else {
        println!("Channel added unsuccessfully with status {}.", status);
        unsafe {
            uni::mrcp_application_session_terminate(session);
        }
    }
}

fn process_channel_remove(session: *mut uni::mrcp_session_t, channel: *mut uni::mrcp_channel_t) {
    unsafe {
        let recog_channel = uni::mrcp_application_channel_object_get(channel) as *mut RecogChannel;
        let _ = Box::from_raw(recog_channel);
        uni::mrcp_application_session_terminate(session);
    }
}

fn create_recognize_message(
    session: *mut uni::mrcp_session_t,
    channel: *mut uni::mrcp_channel_t,
) -> *mut uni::mrcp_message_t {
    let mrcp_message = unsafe {
        uni::mrcp_application_message_create(session, channel, uni::RECOGNIZER_RECOGNIZE as _)
    };
    if !mrcp_message.is_null() {
        unsafe {
            let generic_header =
                uni_lib_client_rs::inline_mrcp_generic_header_prepare(mrcp_message);
            if generic_header.is_null() {
                uni_lib_client_rs::inline_apt_string_assign(
                    &mut (*generic_header).content_type as _,
                    b"text/plain\0".as_ptr() as _,
                    (*mrcp_message).pool,
                );
                uni::mrcp_generic_header_property_add(
                    mrcp_message,
                    uni::GENERIC_HEADER_CONTENT_TYPE as _,
                );
            }
            let recog_header = uni_lib_client_rs::inline_mrcp_resource_header_prepare(mrcp_message)
                as *mut uni::mrcp_recog_header_t;
            if !recog_header.is_null() {
                if (*mrcp_message).start_line.version == uni::MRCP_VERSION_2 {
                    (*recog_header).cancel_if_queue = uni::FALSE;
                    uni::mrcp_resource_header_property_add(
                        mrcp_message,
                        uni::RECOGNIZER_HEADER_CANCEL_IF_QUEUE as _,
                    );
                }
                (*recog_header).no_input_timeout = 1000;
                uni::mrcp_resource_header_property_add(
                    mrcp_message,
                    uni::RECOGNIZER_HEADER_NO_INPUT_TIMEOUT as _,
                );
                (*recog_header).recognition_timeout = 10000;
                uni::mrcp_resource_header_property_add(
                    mrcp_message,
                    uni::RECOGNIZER_HEADER_RECOGNITION_TIMEOUT as _,
                );
                (*recog_header).start_input_timers = uni::TRUE;
                uni::mrcp_resource_header_property_add(
                    mrcp_message,
                    uni::RECOGNIZER_HEADER_START_INPUT_TIMERS as _,
                );
                (*recog_header).confidence_threshold = 0.87;
                uni::mrcp_resource_header_property_add(
                    mrcp_message,
                    uni::RECOGNIZER_HEADER_CONFIDENCE_THRESHOLD as _,
                );
                (*recog_header).speech_complete_timeout = 1600;
                uni::mrcp_resource_header_property_add(
                    mrcp_message,
                    uni::RECOGNIZER_HEADER_SPEECH_COMPLETE_TIMEOUT as _,
                );
            }
            uni_lib_client_rs::inline_apt_string_assign(
                &mut (*mrcp_message).body,
                std::ptr::null() as _,
                (*mrcp_message).pool,
            );
        }
    }
    mrcp_message
}

fn prepare_channel(channel: *mut uni::mrcp_channel_t) {
    unsafe {
        let recog_channel = uni::mrcp_application_channel_object_get(channel) as *mut RecogChannel;
        let path = (*recog_channel).path.as_str();
        let source = std::fs::File::open(path);
        if source.is_err() {
            println!("Could not open the source: {:?}", source);
        }
        (*recog_channel).source = source.ok();
    }
}
