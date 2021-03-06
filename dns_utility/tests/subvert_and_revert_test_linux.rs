// Copyright (c) 2017-2018, Substratum LLC (https://substratum.net) and/or its affiliates. All rights reserved.
#![cfg (target_os = "linux")]
extern crate sub_lib;
extern crate dns_utility_lib;

mod utils;

use std::io;
use std::io::Read;
use std::path::Path;

use dns_utility_lib::resolv_conf_dns_modifier::ResolvConfDnsModifier;
use utils::TestCommand;
use std::fs::File;

#[test]
fn resolv_conf_subvert_and_revert_integration () {
    let contents = match get_file_contents () {
        Ok (c) => c,
        Err (_) => {println! ("---INTEGRATION TEST CANNOT YET RUN IN THIS ENVIRONMENT---"); return}
    };
    let active_nameservers: Vec<String> = ResolvConfDnsModifier::new ().active_nameservers (contents.as_str ()).iter ()
        .map (|entry| entry.0.clone ()).collect ();
    assert_eq! (contents.contains ("\nnameserver 127.0.0.1"), false, "Already contains '\\n#nameserver 127.0.0.1':\n{}", contents);

    let mut subvert_command = TestCommand::start ("dns_utility", vec! ("subvert"));
    let exit_status = subvert_command.wait ();
    assert_eq! (exit_status, Some (0), "{}", subvert_command.output ());

    let contents = get_file_contents ().expect ("Couldn't get file contents after subversion");
    assert_eq! (contents.contains ("\nnameserver 127.0.0.1"), true, "Doesn't contain '\\n#nameserver 127.0.0.1':\n{}", contents);
    active_nameservers.iter ().for_each (|entry| {
        assert_eq! (contents.contains (&format! ("\n#{}", entry)[..]), true, "Doesn't contain '\\n#{}':\n{}", entry, contents)
    });

    let mut revert_command = TestCommand::start ("dns_utility", vec! ("revert"));
    let exit_status = revert_command.wait ();
    assert_eq! (exit_status, Some (0), "{}", revert_command.output ());

    let contents = get_file_contents ().expect ("Couldn't get file contents after reversion");
    assert_eq! (contents.contains ("\nnameserver 127.0.0.1"), false, "Still contains '\\n#nameserver 127.0.0.1':\n{}", contents);
    active_nameservers.iter ().for_each (|entry| {
        assert_eq! (contents.contains (&format! ("\n{}", entry)[..]), true, "Doesn't contain '\\n{}':\n{}", entry, contents)
    });

}

fn get_file_contents () -> io::Result<String> {
    let path = Path::new ("/").join (Path::new ("etc")).join (Path::new ("resolv.conf"));
    let mut file = File::open (path)?;
    let mut contents = String::new ();
    file.read_to_string (&mut contents)?;
    Ok (contents)
}
