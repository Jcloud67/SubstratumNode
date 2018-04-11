// Copyright (c) 2017-2018, Substratum LLC (https://substratum.net) and/or its affiliates. All rights reserved.
use std::borrow::Borrow;
use sub_lib::hopper::IncipientCoresPackage;
use sub_lib::hopper::ExpiredCoresPackage;
use sub_lib::hop::Hop;
use sub_lib::route::Route;
use sub_lib::cryptde::CryptDE;
use sub_lib::cryptde::Key;
use sub_lib::cryptde::CryptData;
use sub_lib::hopper::HopperSubs;
use sub_lib::peer_actors::BindMessage;
use sub_lib::dispatcher::Component;
use actix::Subscriber;
use actix::Actor;
use actix::Context;
use actix::Handler;
use actix::SyncAddress;

pub struct Hopper {
    cryptde: Box<CryptDE>,
    to_proxy_server: Option<Box<Subscriber<ExpiredCoresPackage> + Send>>,
    to_proxy_client: Option<Box<Subscriber<ExpiredCoresPackage> + Send>>,
}

impl Actor for Hopper {
    type Context = Context<Self>;
}

impl Handler<BindMessage> for Hopper {
    type Result = ();

    fn handle(&mut self, msg: BindMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.to_proxy_server = Some(msg.peer_actors.proxy_server.from_hopper);
        self.to_proxy_client = Some(msg.peer_actors.proxy_client.from_hopper);
        ()
    }
}

impl Handler<IncipientCoresPackage> for Hopper {
    type Result = ();

    fn handle(&mut self, msg: IncipientCoresPackage, _ctx: &mut Self::Context) -> Self::Result {
        let (live_package, _key) = LiveCoresPackage::from_incipient(msg, self.cryptde.borrow());
        let next_hop = live_package.next_hop(self.cryptde.borrow());

        match next_hop.component {
            Some(Component::ProxyServer) => {
                let expired_package = live_package.to_expired(self.cryptde.borrow());
                self.to_proxy_server.as_ref().expect("ProxyServer unbound in Hopper").send(expired_package).expect("Proxy Server is dead")
            },
            Some(Component::ProxyClient) => {
                let expired_package = live_package.to_expired(self.cryptde.borrow());
                self.to_proxy_client.as_ref ().expect ("ProxyClient unbound in Hopper").send (expired_package ).expect ("Proxy Client is dead")
            },
            Some(_) => unimplemented!(),
            None => unimplemented!(),
        };
        ()
    }
}

impl Hopper {
    pub fn new (cryptde: Box<CryptDE>) -> Hopper {
        Hopper {
            cryptde,
            to_proxy_server: None,
            to_proxy_client: None,
        }
    }

    pub fn make_subs_from(addr: &SyncAddress<Hopper>) -> HopperSubs {
        HopperSubs {
            bind: addr.subscriber::<BindMessage>(),
            from_hopper_client: addr.subscriber::<IncipientCoresPackage>(),
        }
    }
}

#[derive (Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveCoresPackage {
    pub hops: Vec<CryptData>,
    pub payload: CryptData
}

impl LiveCoresPackage {
    pub fn new (hops: Vec<CryptData>, payload: CryptData) -> LiveCoresPackage {
        LiveCoresPackage {hops, payload}
    }

    pub fn from_incipient (incipient: IncipientCoresPackage, cryptde: &CryptDE) -> (LiveCoresPackage, Key) {
        let encrypted_payload = cryptde.encode (&incipient.payload_destination_key, &incipient.payload).expect ("Encode error");
        let (next_hop, tail) = incipient.route.deconstruct ();
        if next_hop.public_key.is_none () {unimplemented! ()} // can't send over Substratum Network if no destination
        (LiveCoresPackage::new (tail, encrypted_payload), next_hop.public_key.expect ("Internal error"))
    }

    pub fn to_expired (mut self, cryptde: &CryptDE) -> ExpiredCoresPackage {
        let next_hop = LiveCoresPackage::crypt_data_to_hop (&self.hops.remove (0), cryptde);
        let payload = match cryptde.decode (&cryptde.private_key (), &self.payload) {
            Ok (payload) => payload,
            Err (e) => panic! ("{:?}", e)
        };
        let remaining_route = Route::construct (next_hop, self.hops);
        ExpiredCoresPackage::new (remaining_route, payload)
    }

    pub fn next_hop (&self, cryptde: &CryptDE) -> Hop {
        let encrypted_hop = match &self.hops.first () {
            &Some (ref crypt_data) => *crypt_data,
            &None => unimplemented! ()
        };
        LiveCoresPackage::crypt_data_to_hop (encrypted_hop, cryptde)
    }

    fn crypt_data_to_hop (encrypted_hop: &CryptData, cryptde: &CryptDE) -> Hop {
        match Hop::decode (&cryptde.private_key (), cryptde, encrypted_hop) {
            Ok (hop) => hop,
            Err (e) => panic! ("{:?}", e)
        }
    }
}

#[cfg (test)]
mod tests {
    use super::*;
    use std::thread;
    use actix::msgs;
    use actix::Actor;
    use actix::Arbiter;
    use actix::SyncAddress;
    use actix::System;
    use serde_cbor;
    use sub_lib::cryptde::PlainData;
    use sub_lib::cryptde_null::CryptDENull;
    use sub_lib::dispatcher::Component;
    use sub_lib::hop::Hop;
    use sub_lib::hopper::IncipientCoresPackage;
    use sub_lib::hopper::ExpiredCoresPackage;
    use sub_lib::route::Route;
    use sub_lib::route::RouteSegment;
    use test_utils::test_utils::make_peer_actors_from;
    use test_utils::test_utils::PayloadMock;
    use test_utils::test_utils::Recorder;
    use test_utils::test_utils::route_from_proxy_server;
    use test_utils::test_utils::route_to_proxy_client;
    use test_utils::test_utils::route_from_proxy_client;
    use test_utils::test_utils::route_to_proxy_server;

    #[test]
    fn live_cores_package_can_be_constructed_from_scratch () {
        let hops = vec! (CryptData::new (&[1, 2]), CryptData::new (&[3, 4]));
        let payload = CryptData::new (&[5, 6]);

        let subject = LiveCoresPackage::new (hops.clone (), payload.clone ());

        assert_eq! (subject.hops, hops);
        assert_eq! (subject.payload, payload);
    }

    #[test]
    fn live_cores_package_can_be_constructed_from_incipient_cores_package () {
        let cryptde = CryptDENull::new ();
        let key12 = Key::new (&[1, 2]);
        let key34 = Key::new (&[3, 4]);
        let key56 = Key::new (&[5, 6]);
        let payload = PayloadMock::new ();
        let incipient = IncipientCoresPackage::new (
            Route::new(vec! (
                RouteSegment::new (vec! (&key12, &key34), Component::Neighborhood)
            ), &cryptde).unwrap (),
            payload.clone (),
            &key56
        );

        let (subject, next_stop) = LiveCoresPackage::from_incipient (incipient, &cryptde);

        assert_eq! (next_stop, key12);
        assert_eq! (subject.hops, vec! (
            Hop::with_key (&key34).encode (&key12, &cryptde).unwrap (),
            Hop::with_component (Component::Neighborhood).encode (&key34, &cryptde).unwrap (),
        ));
        assert_eq! (subject.payload, cryptde.encode (&key56, &PlainData::new (&serde_cbor::ser::to_vec (&payload).unwrap ())).unwrap ());
    }

    #[test]
    fn release_2_transmit_cores_package_from_proxy_server_calls_proxy_client_directly () {
        let cryptde = CryptDENull::new ();
        let thread_cryptde = cryptde.clone();
        let incipient_package = IncipientCoresPackage::new (
            route_from_proxy_server(&cryptde.public_key(), &cryptde),
            PayloadMock::new (), &cryptde.public_key ()
        );
        let proxy_client = Recorder::new ();
        let proxy_client_recording = proxy_client.get_recording();
        let proxy_client_awaiter = proxy_client.get_awaiter();

        thread::spawn(move || {
            let system = System::new("release_2_transmit_cores_package_from_proxy_server_calls_proxy_client_directly");
            let peer_actors = make_peer_actors_from(None, None, None, Some(proxy_client));
            let subject = Hopper::new (Box::new (thread_cryptde));
            let subject_addr: SyncAddress<_> = subject.start();
            subject_addr.send(BindMessage { peer_actors });

            subject_addr.send(incipient_package );

            system.run();
        });

        let expected_expired_package = ExpiredCoresPackage::new (
            route_to_proxy_client (&cryptde.public_key (), &cryptde),
            PlainData::new (&serde_cbor::ser::to_vec (&PayloadMock::new ()).unwrap ()[..])
        );

        proxy_client_awaiter.await_message_count(1);
        let recording = proxy_client_recording.lock().unwrap();
        let record = recording.get_record::<ExpiredCoresPackage>(0);
        assert_eq!(record, &expected_expired_package);
    }
    
    #[test]
    #[should_panic (expected = "ProxyServer unbound in Hopper")]
    fn panics_if_proxy_server_is_unbound() {
        let system = System::new("panics_if_proxy_server_is_unbound");
        let cryptde = CryptDENull::new ();
        let incipient_package = IncipientCoresPackage::new (
            route_from_proxy_client(&cryptde.public_key (), &cryptde),
            PayloadMock::new (), &cryptde.public_key ()
        );
        let subject = Hopper::new (Box::new (cryptde));
        let subject_addr: SyncAddress<_> = subject.start();

        subject_addr.send(incipient_package );

        Arbiter::system().send(msgs::SystemExit(0));
        system.run();
    }

    #[test]
    #[should_panic (expected = "ProxyClient unbound in Hopper")]
    fn panics_if_proxy_client_is_unbound() {
        let system = System::new("panics_if_proxy_client_is_unbound");
        let cryptde = CryptDENull::new ();
        let incipient_package = IncipientCoresPackage::new (
            route_from_proxy_server(&cryptde.public_key(), &cryptde),
            PayloadMock::new (), &cryptde.public_key ()
        );
        let subject = Hopper::new (Box::new (cryptde));
        let subject_addr: SyncAddress<_> = subject.start();

        subject_addr.send(incipient_package );

        Arbiter::system().send(msgs::SystemExit(0));
        system.run();
    }

    #[test]
    fn release_2_transmit_cores_package_from_proxy_client_calls_proxy_server_via_subscriber () {
        let cryptde = CryptDENull::new ();
        let thread_cryptde = cryptde.clone();
        let proxy_server = Recorder::new();
        let proxy_server_log_arc = proxy_server.get_recording();
        let proxy_server_awaiter = proxy_server.get_awaiter();
        let incipient_package = IncipientCoresPackage::new (
            route_from_proxy_client(&cryptde.public_key (), &cryptde),
            PayloadMock::new (), &cryptde.public_key ()
        );
        let thread_package = incipient_package.clone();
        thread::spawn(move || {
            let system = System::new("release_2_transmit_cores_package_from_proxy_client_calls_proxy_server_directly");
            let peer_actors = make_peer_actors_from(Some(proxy_server), None, None, None);
            let subject = Hopper::new (Box::new (thread_cryptde));
            let subject_addr: SyncAddress<_> = subject.start();
            subject_addr.send(BindMessage { peer_actors });

            subject_addr.send(thread_package );

            system.run();
        });

        let expected_package = ExpiredCoresPackage::new (
            route_to_proxy_server (&cryptde.public_key (), &cryptde),
            incipient_package.payload
        );

        proxy_server_awaiter.await_message_count(1);
        let recording = proxy_server_log_arc.lock().unwrap();
        let record = recording.get_record::<ExpiredCoresPackage>(0);
        assert_eq!(record, &expected_package);
    }
}
