//! Protocol state machines.
//!
//! These state machines handle all state transitions independently of the
//! connection. Instead of executing side effects (like sending a response
//! message to the peer through the websocket), a `HandleAction` is returned.
//!
//! This allows for better decoupling between protocol logic and network code,
//! and makes it possible to easily add tests.

use boxes::{ByteBox, OpenBox};
use messages::{Message, ClientHello, ClientAuth};
use nonce::{Nonce, Sender, Receiver};
use keystore::{KeyStore, PublicKey};

mod types;
mod state;

pub use self::types::{Role, HandleAction};
use self::state::{ServerHandshakeState, StateTransition};


/// All signaling related data.
pub struct Signaling {
    pub role: Role,
    pub server: ServerContext,
    pub permanent_key: KeyStore,
}

impl Signaling {
    pub fn new(role: Role, permanent_key: KeyStore) -> Self {
        Signaling {
            role: role,
            server: ServerContext::new(),
            permanent_key: permanent_key,
        }
    }

    /// Handle an incoming message.
    pub fn handle_message(&mut self, bbox: ByteBox) -> Vec<HandleAction> {
        // Do the state transition
        let transition = self.next_state(bbox);
        trace!("Server handshake state transition: {:?} -> {:?}", self.server.handshake_state, transition.state);
        self.server.handshake_state = transition.state;

        // Return the action
        transition.actions
    }

    /// Determine the next state based on the incoming message bytes and the
    /// current (read-only) state.
    fn next_state(&self, bbox: ByteBox) -> StateTransition<ServerHandshakeState> {
        // Decode message
        let obox: OpenBox = match self.server.handshake_state {

            // If we're in state `New`, message must be unencrypted.
            ServerHandshakeState::New => {
                match bbox.decode() {
                    Ok(obox) => obox,
                    Err(e) => return ServerHandshakeState::Failure(format!("{}", e)).into(),
                }
            },

            // If we're already in `Failure` state, stay there.
            ServerHandshakeState::Failure(ref msg) => return ServerHandshakeState::Failure(msg.clone()).into(),

            // Otherwise, not yet implemented!
            _ => return ServerHandshakeState::Failure("Not yet implemented".into()).into(),

        };

        match (&self.server.handshake_state, obox.message) {

            // Valid state transitions
            (&ServerHandshakeState::New, Message::ServerHello(msg)) => {
                info!("Hello from server");

                let mut actions = Vec::with_capacity(3);

                trace!("Server key is {:?}", msg.key);
                actions.push(HandleAction::SetServerKey(msg.key.clone()));

                // Reply with client-hello message
                let key = self.permanent_key.public_key().clone();
                let client_hello = ClientHello::new(key).into_message();
                let client_hello_nonce = Nonce::new(
                    [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
                    Sender::new(0),
                    Receiver::new(0),
                    0,
                    123,
                );
                let reply = OpenBox::new(client_hello, client_hello_nonce);
                actions.push(HandleAction::Reply(reply.encode()));

                // Send with client-auth message
                let client_auth = ClientAuth {
                    your_cookie: [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0], // TODO
                    subprotocols: vec!["vX.saltyrtc.org".into()], // TODO
                    ping_interval: 0, // TODO
                    your_key: None, // TODO
                }.into_message();
                let client_auth_nonce = Nonce::new(
                    [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
                    Sender::new(0),
                    Receiver::new(0),
                    0,
                    124,
                );
                let reply = OpenBox::new(client_auth, client_auth_nonce);
                actions.push(HandleAction::Reply(reply.encode()));

                // TODO: Can we prevent confusing an incoming and an outgoing nonce?
                StateTransition {
                    state: ServerHandshakeState::ClientInfoSent,
                    actions: actions,
                }
            },

            // A failure transition is terminal and does not change
            (&ServerHandshakeState::Failure(ref msg), _) => ServerHandshakeState::Failure(msg.clone()).into(),

            // Any undefined state transition changes to Failure
            (s, message) => {
                ServerHandshakeState::Failure(
                    format!("Invalid event transition: {:?} <- {}", s, message.get_type())
                ).into()
            }

        }
    }
}


trait PeerContext {
    fn address(&self) -> Receiver;
    fn permanent_key(&self) -> Option<&PublicKey>;
    fn session_key(&self) -> Option<&PublicKey>;
}

pub struct ServerContext {
    handshake_state: ServerHandshakeState,
    permanent_key: Option<PublicKey>,
    session_key: Option<PublicKey>,
}

impl ServerContext {
    pub fn new() -> Self {
        ServerContext {
            handshake_state: ServerHandshakeState::New,
            permanent_key: None,
            session_key: None,
        }
    }
}

impl PeerContext for ServerContext {
    fn address(&self) -> Receiver {
        Receiver::server()
    }

    fn permanent_key(&self) -> Option<&PublicKey> {
        self.permanent_key.as_ref()
    }

    fn session_key(&self) -> Option<&PublicKey> {
        self.session_key.as_ref()
    }
}


#[cfg(test)]
mod tests {
    use ::messages::{ServerHello, ClientHello};
    use super::*;

    fn create_test_nonce() -> Nonce {
        Nonce::new(
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            Sender::new(17),
            Receiver::new(18),
            258,
            50_595_078,
        )
    }

    fn create_test_bbox() -> ByteBox {
        ByteBox::new(vec![1, 2, 3], create_test_nonce())
    }

    /// Test that states and tuples implement Into<ServerHandshakeState>.
    #[test]
    fn server_handshake_state_from() {
        let t1: StateTransition<_> = StateTransition::new(ServerHandshakeState::New, vec![HandleAction::Reply(create_test_bbox())]);
        let t2: StateTransition<_> = StateTransition::new(ServerHandshakeState::New, vec![HandleAction::Reply(create_test_bbox())]).into();
        let t3: StateTransition<_> = (ServerHandshakeState::New, HandleAction::Reply(create_test_bbox())).into();
        let t4: StateTransition<_> = (ServerHandshakeState::New, vec![HandleAction::Reply(create_test_bbox())]).into();
        assert_eq!(t1, t2);
        assert_eq!(t1, t3);
        assert_eq!(t1, t4);

        let t4: StateTransition<_> = ServerHandshakeState::New.into();
        let t5: StateTransition<_> = StateTransition::new(ServerHandshakeState::New, vec![]);
        assert_eq!(t4, t5);
    }

//    #[test]
//    fn transition_server_hello() {
//        // Create a new initial state.
//        let state = ServerHandshakeState::New;
//        assert_eq!(state, ServerHandshakeState::New);
//
//        // Transition to `ClientInfoSent` state.
//        let msg = Message::ServerHello(ServerHello::random());
//        let obox = OpenBox::new(msg, Nonce::random());
//        let StateTransition { state, action } = state.next(obox.encode(), Role::Initiator);
//        assert_eq!(state, ServerHandshakeState::ClientInfoSent);
//        match action {
//            HandleAction::Reply(..) => (),
//            a @ _ => panic!("Invalid action: {:?}", a)
//        };
//    }

//    #[test]
//    fn transition_failure() {
//        // Create a new initial state.
//        let state = ServerHandshakeState::New;
//        assert_eq!(state, ServerHandshakeState::New);
//
//        // Invalid transition to client-hello state.
//        let msg = Message::ClientHello(ClientHello::random());
//        let obox = OpenBox::new(msg, Nonce::random());
//        let StateTransition { state, action } = state.next(obox.encode(), Role::Initiator);
//        assert_eq!(state, ServerHandshakeState::Failure("Invalid event transition: New <- client-hello".into()));
//        assert_eq!(action, HandleAction::None);
//
//        // Another invalid transition won't change the message
//        let msg = Message::ServerHello(ServerHello::random());
//        let obox = OpenBox::new(msg, Nonce::random());
//        let StateTransition { state, action } = state.next(obox.encode(), Role::Initiator);
//        assert_eq!(state, ServerHandshakeState::Failure("Invalid event transition: New <- client-hello".into()));
//        assert_eq!(action, HandleAction::None);
//    }

    #[test]
    fn server_context_new() {
        let ctx = ServerContext::new();
        assert_eq!(ctx.address(), Receiver::new(0));
        assert_eq!(ctx.permanent_key(), None);
        assert_eq!(ctx.session_key(), None);
    }
}