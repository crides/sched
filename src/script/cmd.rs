use std::collections::HashMap;
use std::iter;
use std::sync::Mutex;

use clap::App;
use gluon::{
    vm::{
        api::{OwnedFunction, IO},
        ExternModule,
    },
    Thread,
};
use rustyline::{error::ReadlineError, Editor};

#[derive(Clone, Debug, Userdata, Trace, VmType)]
#[gluon_userdata(clone)]
#[gluon(vm_type = "cmd.ArgMatches")]
#[gluon_trace(skip)]
struct ArgMatches(clap::ArgMatches<'static>);

type CommandHandler = OwnedFunction<fn(ArgMatches) -> IO<()>>;

thread_local! {
    static CMDS: Mutex<(Option<App<'static, 'static>>, HashMap<String, CommandHandler>)> = Mutex::new((Some(App::new("cmd")), HashMap::new()));
}

fn add_command(name: String, usage: String, handler: CommandHandler) {
    let sub = App::new(name.clone()).args_from_usage(Box::leak(Box::new(usage)));
    CMDS.with(|c| {
        let mut cmds = c.lock().unwrap();
        let app = cmds.0.take().unwrap();
        cmds.0 = Some(app.subcommand(sub));
        cmds.1.insert(name, handler);
    });
}

pub fn cmd_repl() -> bool {
    let mut editor = Editor::<()>::new();
    loop {
        match editor.readline(">=> ") {
            Ok(line) => {
                let line = line.trim();
                if line == "repl" {
                    return true;
                }
                let args = line.split_ascii_whitespace();
                CMDS.with(|c| {
                    let mut cmds = c.lock().unwrap();
                    match cmds
                        .0
                        .as_mut()
                        .unwrap()
                        .get_matches_from_safe_borrow(iter::once("cmd").chain(args))
                    {
                        Ok(matches) => {
                            let (name, submatches) = matches.subcommand();
                            let _ = cmds
                                .1
                                .get_mut(name)
                                .unwrap()
                                .call(ArgMatches(submatches.unwrap().clone()));
                        }
                        Err(e) => {
                            eprintln!("{}", e.message);
                            eprintln!("kind: {:?}, extra: {:?}", e.kind, e.info);
                        }
                    }
                });
            }
            Err(ReadlineError::Eof) => {
                return false;
            }
            Err(e) => {
                eprintln!("{:?}", e);
                return false;
            }
        }
    }
}

fn value_of<'a>(m: &'a ArgMatches, name: &str) -> Option<&'a str> {
    m.0.value_of(name)
}

fn values_of<'a>(m: &'a ArgMatches, name: &str) -> Vec<&'a str> {
    m.0.values_of(name).map(|v| v.collect()).unwrap_or_default()
}

pub fn load(thread: &Thread) -> Result<ExternModule, gluon::vm::Error> {
    thread.register_type::<ArgMatches>("cmd.ArgMatches", &[])?;
    ExternModule::new(
        thread,
        record! {
            add_command => primitive!(3, add_command),
            // cmd_repl => primitive!(1, cmd_repl),
            value_of => primitive!(2, value_of),
            values_of => primitive!(2, values_of),
        },
    )
}