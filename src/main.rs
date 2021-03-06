#[macro_use]
extern crate gluon_codegen;
#[macro_use]
extern crate gluon;
#[macro_use]
extern crate serde_derive;

mod repl;
mod script;
mod signal;
mod storage;
mod util;

use std::fs;
use std::path::PathBuf;

use clap::{App, Arg};
use dirs::config_dir;

use util::print_gluon_err;

fn main() {
    let config_dir = config_dir().unwrap().join("sched");
    if !config_dir.is_dir() {
        fs::create_dir_all(&config_dir).unwrap();
    }
    let matches = App::new("sched")
        .arg(Arg::with_name("init-file").required(false))
        .get_matches();
    let init_file: PathBuf = matches
        .value_of("init-file")
        .map_or_else(|| config_dir.join("init.glu"), |s| s.into());
    let vm = script::get_vm(config_dir);
    if let Err(e) = script::run_user(&vm, &init_file) {
        print_gluon_err(e);
        return;
    }
    if script::cmd::cmd_repl() {
        let res = repl::run(&vm, "> ");
        if let Err(e) = res {
            print_gluon_err(e);
        }
    }
}
