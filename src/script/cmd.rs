use std::collections::HashMap;
use std::iter;
use std::sync::Mutex;

use clap::App;
use dirs::data_dir;
use gluon::{
    vm::{
        api::{OwnedFunction, IO},
        ExternModule,
    },
    Thread,
};
use rustyline::{error::ReadlineError, Editor};

use crate::util::print_gluon_err;

#[derive(Clone, Debug, Userdata, Trace, VmType)]
#[gluon_userdata(clone)]
#[gluon(vm_type = "cmd.ArgMatches")]
#[gluon_trace(skip)]
struct ArgMatches(clap::ArgMatches<'static>);

type CommandHandler = OwnedFunction<fn(ArgMatches) -> IO<()>>;

thread_local! {
    static CMDS: Mutex<(Option<App<'static, 'static>>, HashMap<String, CommandHandler>)> = Mutex::new((Some(App::new("cmd")), HashMap::new()));
}

fn cmd(name: String, usage: String, handler: CommandHandler) -> IO<()> {
    let sub = App::new(name.clone()).args_from_usage(Box::leak(Box::new(usage)));
    CMDS.with(|c| {
        let mut cmds = c.lock().unwrap();
        let app = cmds.0.take().unwrap();
        cmds.0 = Some(app.subcommand(sub));
        cmds.1.insert(name, handler);
    });
    IO::Value(())
}

// FIXME make the process a long lasting process (daemon), so that the client can use the shell's parsing to
// send commands to the daemon and get response back. Or implement a nushell plugin, same idea
pub fn cmd_repl() -> bool {
    let mut editor = Editor::<()>::new();
    if let Some(d) = data_dir() {
        let _ = editor.load_history(&d.join("sched/history"));
    }
    let ret = loop {
        match editor.readline(">=> ") {
            Ok(line) => {
                let line = line.trim();
                if !line.is_empty() {
                    editor.add_history_entry(line);
                }
                if line == "repl" {
                    break true;
                }
                let args = line.split_ascii_whitespace();
                let res = CMDS.with(|c| {
                    let mut cmds = c.lock().unwrap();
                    match cmds
                        .0
                        .as_mut()
                        .unwrap()
                        .get_matches_from_safe_borrow(iter::once("cmd").chain(args))
                    {
                        Ok(matches) => {
                            let (name, submatches) = matches.subcommand();
                            if name.len() != 0 {
                                let res = cmds
                                    .1
                                    .get_mut(name)
                                    .unwrap()
                                    .call(ArgMatches(submatches.unwrap().clone()));
                                if let Err(e) = res {
                                    eprintln!("Error running command handler:");
                                    print_gluon_err(e.into());
                                    return false;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("{}", e.message);
                        }
                    }
                    true
                });
                if !res {
                    break false;
                }
            }
            Err(ReadlineError::Eof) => {
                break false;
            }
            Err(e) => {
                eprintln!("{:?}", e);
                break false;
            }
        }
    };
    if let Some(d) = data_dir() {
        editor.save_history(&d.join("sched/history")).unwrap();
    }
    ret
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
            cmd => primitive!(3, cmd),
            value_of => primitive!(2, value_of),
            values_of => primitive!(2, values_of),
        },
    )
}
