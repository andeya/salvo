// Copyright (c) 2018-2020 Sean McArthur
// Licensed under the MIT license http://opensource.org/licenses/MIT
//
// port from https://github.com/seanmonstar/warp/blob/master/examples/sse_chat.rs

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use salvo::extra::sse::{SseEvent, SseKeepAlive};
use salvo::prelude::*;

type Users = Mutex<HashMap<usize, mpsc::UnboundedSender<Message>>>;

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
static ONLINE_USERS: Lazy<Users> = Lazy::new(|| Users::default());

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let router = Router::new().handle(index).push(
        Router::with_path("chat")
            .get(user_connected)
            .push(Router::with_path("<id>").post(chat_send)),
    );
    tracing::info!("Listening on http://0.0.0.0:7878");
    Server::new(TcpListener::bind("0.0.0.0:7878")).serve(router).await;
}

#[derive(Debug)]
enum Message {
    UserId(usize),
    Reply(String),
}

#[fn_handler]
async fn chat_send(req: &mut Request, res: &mut Response) {
    let my_id = req.get_param::<usize>("id").unwrap();
    let msg = req.read_text().await.unwrap();
    user_message(my_id, msg);
    res.set_status_code(StatusCode::OK);
}

#[fn_handler]
async fn user_connected(_req: &mut Request, res: &mut Response) {
    // Use a counter to assign a new unique ID for this user.
    let my_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);

    tracing::info!("new chat user: {}", my_id);

    // Use an unbounded channel to handle buffering and flushing of messages
    // to the event source...
    let (tx, rx) = mpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);

    tx.send(Message::UserId(my_id))
        // rx is right above, so this cannot fail
        .unwrap();

    // Save the sender in our list of connected users.
    ONLINE_USERS.lock().unwrap().insert(my_id, tx);

    // Convert messages into Server-Sent Events and returns resulting stream.
    let stream = rx.map(|msg| match msg {
        Message::UserId(my_id) => Ok::<_, salvo::Error>(SseEvent::default().name("user").data(my_id.to_string())),
        Message::Reply(reply) => Ok(SseEvent::default().data(reply)),
    });
    SseKeepAlive::new(stream).streaming(res);
}

fn user_message(my_id: usize, msg: &str) {
    let new_msg = format!("<User#{}>: {}", my_id, msg);

    // New message from this user, send it to everyone else (except same uid)...
    //
    // We use `retain` instead of a for loop so that we can reap any user that
    // appears to have disconnected.
    ONLINE_USERS.lock().unwrap().retain(|uid, tx| {
        if my_id == *uid {
            // don't send to same user, but do retain
            true
        } else {
            // If not `is_ok`, the SSE stream is gone, and so don't retain
            tx.send(Message::Reply(new_msg.clone())).is_ok()
        }
    });
}

#[fn_handler]
async fn index(res: &mut Response) {
    res.render(Text::Html(INDEX_HTML));
}

static INDEX_HTML: &str = r#"
<!DOCTYPE html>
<html>
    <head>
        <title>SSE Chat</title>
    </head>
    <body>
        <h1>SSE Chat</h1>
        <div id="chat">
            <p><em>Connecting...</em></p>
        </div>
        <input type="text" id="msg" />
        <button type="button" id="submit">Send</button>
        <script>
        const chat = document.getElementById('chat');
        const msg = document.getElementById('msg');
        const submit = document.getElementById('submit');
        let sse = new EventSource(`http://${location.host}/chat`);
        sse.onopen = function() {
            chat.innerHTML = "<p><em>Connected!</em></p>";
        }
        var userId;
        sse.addEventListener("user", function(msg) {
            userId = msg.data;
        });
        sse.onmessage = function(msg) {
            showMessage(msg.data);
        };
        document.getElementById('submit').onclick = function() {
            var msg = text.value;
            var xhr = new XMLHttpRequest();
            xhr.open("POST", `${uri}/${user_id}`, true);
            xhr.send(msg);
            text.value = '';
            showMessage('<You>: ' + msg);
        };
        function showMessage(data) {
            const line = document.createElement('p');
            line.innerText = data;
            chat.appendChild(line);
        }
        </script>
    </body>
</html>
"#;