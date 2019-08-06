#[macro_use]
extern crate log;
extern crate env_logger;
extern crate tokio;
extern crate tokio_xmpp;
extern crate xmpp_parsers;
extern crate rpassword;
extern crate futures;
extern crate minidom;
#[macro_use]
extern crate derive_error;
extern crate tokio_file_unix;

use std::convert::TryFrom;
use std::rc::Rc;
use std::sync::mpsc;
use futures::{future, Future, Sink, Stream};
use tokio::runtime::current_thread::Runtime;
use tokio::io::{stdin};
use tokio::codec::FramedRead;
use tokio_xmpp::{Client, Packet};
use xmpp_parsers::message::{Message, MessageType};
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::carbons;
use xmpp_parsers::Element;

mod core;
mod plugins;

use plugins::{Plugin, PluginManager};
use plugins::ui::CommandCodec;

fn main_loop(mgr: Rc<PluginManager>) {
    let mut rt = Runtime::new().unwrap();
    let mgr = Rc::clone(&mgr);

    let file = tokio_file_unix::raw_stdin().unwrap();
    let file = tokio_file_unix::File::new_nb(file).unwrap();
    let file = file.into_reader(&tokio::reactor::Handle::current()).unwrap();

    let ui = FramedRead::new(file, CommandCodec::new()).for_each(move |command| {
        let mgr = Rc::clone(&mgr);
        match command.command.as_ref() {
            "/connect\n" => {
                let account = "needle@trashserver.net";
                info!("Connecting to {}", account);
                let client = Client::new(account, "pass").unwrap();

                let (mut sink, stream) = client.split();

                let client = stream.for_each(move |event| {
                    let mgr = Rc::clone(&mgr);
                    if event.is_online() {
                        info!("Connected as {}", account);

                        mgr.on_connect(&mut sink);

                        let mut presence = Presence::new(PresenceType::None);
                        presence.show = Some(PresenceShow::Chat);

                        sink.start_send(Packet::Stanza(presence.into())).unwrap();
                    } else if let Some(stanza) = event.into_stanza() {
                        trace!("RECV: {}", String::from(&stanza));

                        handle_stanza(mgr, stanza);
                    }

                    future::ok(())
                }).map_err(|e| {
                    println!("Err: {:?}", e);
                });

                tokio::runtime::current_thread::spawn(client);

            },
            _ => {
                println!("Unknown command {}", command.command);
            }
        }

        Ok(())
    });

    let res = rt.block_on(ui);
    println!("! {:?}", res);
}

fn handle_stanza(mgr: Rc<PluginManager>, stanza: Element) {
    if let Some(message) = Message::try_from(stanza).ok() {
        handle_message(mgr, message);
    }
}

fn handle_message(mgr: Rc<PluginManager>, message: Message) {
    let from = match message.from {
        Some(from) => from,
        None => return,
    };

    if let Some(ref body) = message.bodies.get("") {
        if message.type_ != MessageType::Error {
            let mut message = core::Message::new(from.clone(), body.0.clone());
            mgr.on_message(&mut message);
        }
    }

    for payload in message.payloads {
        if let Some(received) = carbons::Received::try_from(payload).ok() {
            if let Some(ref original) = received.forwarded.stanza {
                if message.type_ != MessageType::Error {
                    if let Some(body) = original.bodies.get("") {
                        let mut message = core::Message::new(from.clone(), body.0.clone());
                        mgr.on_message(&mut message);
                    }
                }
            }
        }
    }
}

fn main() {
    env_logger::init();

    let mut plugin_manager = PluginManager::new();
    plugin_manager.add::<plugins::disco::Disco>(Box::new(plugins::disco::Disco::new())).unwrap();
    plugin_manager.add::<plugins::carbons::CarbonsPlugin>(Box::new(plugins::carbons::CarbonsPlugin::new())).unwrap();
    plugin_manager.add::<plugins::ui::UIPlugin>(Box::new(plugins::ui::UIPlugin::new())).unwrap();

    plugin_manager.init().unwrap();

    main_loop(Rc::new(plugin_manager))
}
