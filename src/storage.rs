use std::collections::HashMap;

use bson::{doc, document::ValueAccessError, from_bson, Bson, Document};
use chrono::{DateTime, Utc};
use mongodb::sync::{Client, Collection};
use rlua::prelude::*;

use crate::event::EventHandlers;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Invalid regex patter: '{0}'")]
    Regex(String),
    #[error("No such key in attributes: '{0}'")]
    InvalidKey(String),
    #[error("Invalid log ID '{0}'")]
    InvalidLogID(i32),
    #[error("Invalid object ID '{0}'")]
    InvalidObjID(i32),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Log {
    #[serde(rename(deserialize = "type"))]
    pub typ: String,
    pub time: DateTime<Utc>,
    #[serde(default)]
    pub attrs: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    #[serde(rename(deserialize = "type"))]
    pub typ: String,
    pub desc: Option<String>,
    #[serde(default)]
    pub deps: Vec<ObjectRef>,
    #[serde(default)]
    pub subs: Vec<ObjectRef>,
    #[serde(default)]
    pub refs: Vec<ObjectRef>,
    #[serde(default)]
    pub attrs: HashMap<String, String>,
}

pub type ObjectRef = i32;

pub struct Storage<'lua> {
    ids: Collection,
    logs: Collection,
    objs: Collection,
    handlers: EventHandlers<'lua>,
}

impl<'lua> Storage<'lua> {
    pub fn new() -> Storage<'lua> {
        let client =
            Client::with_uri_str("mongodb://localhost:27017/").expect("Can't connect to server");
        let db = client.database("sched");
        let ids = db.collection("ids");
        if ids
            .find_one(doc! { "_id": "logs_id" }, None)
            .unwrap()
            .is_none()
        {
            ids.insert_one(doc! { "_id": "logs_id", "id": 1i32 }, None)
                .unwrap();
        }
        if ids
            .find_one(doc! { "_id": "objs_id" }, None)
            .unwrap()
            .is_none()
        {
            ids.insert_one(doc! { "_id": "objs_id", "id": 1i32 }, None)
                .unwrap();
        }

        Storage {
            ids,
            logs: db.collection("logs"),
            objs: db.collection("objs"),
            handlers: EventHandlers::new(),
        }
    }

    pub fn add_lua(&mut self, pat: &str, f: LuaFunction<'lua>) -> Result<()> {
        self.handlers.add_lua(pat, f)
    }

    pub fn create_log(&mut self, typ: &str, attrs: Document) -> Result<i32> {
        let id = self
            .ids
            .find_one_and_update(
                doc! { "_id": "logs_id" },
                doc! { "$inc": { "id": 1 } },
                None,
            )
            .unwrap()
            .unwrap()
            .get_i32("id")
            .unwrap();
        // let attrs = attrs.into_iter().map(|(k, v)| (k, Bson::String(v))).collect::<Document>();
        if attrs.len() > 0 {
            self.logs
                .insert_one(
                    doc! { "_id": id, "type": typ, "time": Utc::now(), "attrs": attrs },
                    None,
                )
                .unwrap();
        } else {
            self.logs
                .insert_one(doc! { "_id": id, "type": typ, "time": Utc::now() }, None)
                .unwrap();
        }

        // FIXME optimize this
        let log = self.get_log(id)?;
        self.handlers.handle(&log);
        Ok(id)
    }

    pub fn log_set_attr(&mut self, id: i32, key: &str, val: &str) -> Result<()> {
        if key.contains('.') {
            return Err(Error::InvalidKey(key.to_string()));
        }
        let key = format!("attrs.{}", key);
        self.logs
            .find_one_and_update(
                doc! { "_id": id, key.clone(): { "$exists": false } },
                doc! { "$set": { key.clone(): val } },
                None,
            )
            .unwrap();
        self.create_log("log.set_attr", doc! { "id": id, "attr": key })?;
        Ok(())
    }

    pub fn get_log(&mut self, id: i32) -> Result<Log> {
        let log = self
            .logs
            .find_one(doc! { "_id": id }, None)
            .unwrap()
            .ok_or_else(|| Error::InvalidLogID(id))?;
        // FIXME The deser impl in `Bson` is missing for `Datetime<>`.
        // Github issue: https://github.com/mongodb/bson-rust/issues/191, and
        // tracking Jira in MongoDB: https://jira.mongodb.org/browse/RUST-506
        Ok(Log {
            typ: log.get_str("type").unwrap().into(),
            time: log.get_datetime("time").unwrap().clone(),
            attrs: log
                .get_document("attrs")
                .map(|d| {
                    d.into_iter()
                        .map(|(k, v)| {
                            let v = match v {
                                Bson::Int32(i) => i.to_string(),
                                Bson::String(s) => s.to_string(),
                                _ => unreachable!(),
                            };
                            (k.clone(), v)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    pub fn create_obj(&mut self, name: &str, typ: &str) -> Result<i32> {
        let id = self
            .ids
            .find_one_and_update(
                doc! { "_id": "objs_id" },
                doc! { "$inc": { "id": 1 } },
                None,
            )
            .unwrap()
            .unwrap()
            .get_i32("id")
            .unwrap();
        self.objs
            .insert_one(doc! { "_id": id, "name": name, "type": typ }, None)
            .unwrap();
        self.create_log("obj.create", doc! { "id": id })?;
        Ok(id)
    }

    pub fn obj_set_desc(&mut self, id: i32, desc: &str) -> Result<()> {
        let old_obj = self
            .objs
            .find_one_and_update(doc! { "_id": id }, doc! { "$set": { "desc": desc } }, None)
            .unwrap()
            .unwrap();
        let attrs = match old_obj.get_str("desc") {
            Ok(old) => {
                doc! { "id": id, "old": old, "new": desc }
            }
            Err(ValueAccessError::NotPresent) => {
                doc! { "id": id, "new": desc }
            }
            _ => unreachable!(),
        };
        self.create_log("obj.set_desc", attrs)?;
        Ok(())
    }

    pub fn obj_add_dep(&mut self, id: i32, dep: i32) -> Result<()> {
        self.objs
            .find_one_and_update(
                doc! { "_id": id },
                doc! { "$addToSet": { "deps": dep } },
                None,
            )
            .unwrap();
        self.create_log("obj.add_dep", doc! { "id": id, "dep": dep })?;
        Ok(())
    }

    pub fn obj_add_sub(&mut self, id: i32, sub: i32) -> Result<()> {
        self.objs
            .find_one_and_update(
                doc! { "_id": id },
                doc! { "$addToSet": { "subs": sub } },
                None,
            )
            .unwrap();
        self.create_log("obj.add_sub", doc! { "sub": sub, "id": id })?;
        Ok(())
    }

    pub fn obj_add_ref(&mut self, id: i32, rf: i32) -> Result<()> {
        self.objs
            .find_one_and_update(
                doc! { "_id": id },
                doc! { "$addToSet": { "refs": rf } },
                None,
            )
            .unwrap();
        self.create_log("obj.add_ref", doc! { "ref": rf, "id": id })?;
        Ok(())
    }

    pub fn obj_set_attr(&mut self, id: i32, key: &str, val: &str) -> Result<()> {
        if key.contains('.') {
            return Err(Error::InvalidKey(key.to_string()));
        }
        let old_obj = self
            .objs
            .find_one_and_update(
                doc! { "_id": id },
                doc! { "$set": { format!("attrs.{}", key): val } },
                None,
            )
            .unwrap()
            .unwrap();
        let attrs = match old_obj
            .get_document("attrs")
            .map(|d| d.get_str(key).unwrap())
        {
            Ok(old) => {
                doc! { "key": key, "id": id, "old": old, "new": val }
            }
            Err(ValueAccessError::NotPresent) => {
                doc! { "key": key, "id": id, "new": val }
            }
            _ => unreachable!(),
        };
        self.create_log("obj.set_attr", attrs)?;
        Ok(())
    }

    pub fn obj_del_dep(&mut self, id: i32, dep: i32) -> Result<()> {
        self.objs
            .find_one_and_update(doc! { "_id": id }, doc! { "$pull": { "deps": dep } }, None)
            .unwrap();
        self.create_log("obj.del_dep", doc! { "dep": dep, "id": id })?;
        Ok(())
    }

    pub fn obj_del_sub(&mut self, id: i32, sub: i32) -> Result<()> {
        self.objs
            .find_one_and_update(doc! { "_id": id }, doc! { "$pull": { "subs": sub } }, None)
            .unwrap();
        self.create_log("obj.del_sub", doc! { "sub": sub, "id": id })?;
        Ok(())
    }

    pub fn obj_del_ref(&mut self, id: i32, rf: i32) -> Result<()> {
        self.objs
            .find_one_and_update(doc! { "_id": id }, doc! { "$pull": { "refs": rf } }, None)
            .unwrap();
        self.create_log("obj.del_ref", doc! { "ref": rf, "id": id })?;
        Ok(())
    }

    pub fn obj_del_attr(&mut self, id: i32, key: &str) -> Result<()> {
        if key.contains('.') {
            return Err(Error::InvalidKey(key.to_string()));
        }
        let old_obj = self
            .objs
            .find_one_and_update(
                doc! { "_id": id },
                doc! { "$unset": { format!("attrs.{}", key): 0 } },
                None,
            )
            .unwrap()
            .unwrap();
        match old_obj
            .get_document("attrs")
            .map(|d| d.get_str(key).unwrap())
        {
            Ok(old) => {
                self.create_log("obj.del_attr", doc! { "id": id, "key": key, "old": old })?;
            }
            _ => (),
        }
        Ok(())
    }

    pub fn get_obj(&mut self, id: i32) -> Result<Object> {
        let obj = self
            .objs
            .find_one(doc! { "_id": id }, None)
            .unwrap()
            .ok_or_else(|| Error::InvalidObjID(id))?;
        Ok(from_bson(obj.into()).unwrap())
    }
}
