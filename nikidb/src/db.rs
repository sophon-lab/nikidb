use crate::datafile;
use crate::datafile::DataFile;
use crate::datafile::Entry;
use crate::error::IoResult;
use crate::option::DataType;
use crate::option::Options;
use crate::option::{DATA_TYPE_HASH, DATA_TYPE_LIST, DATA_TYPE_SET, DATA_TYPE_STR, DATA_TYPE_ZSET};
use crate::result_skip_fail;
use crate::util::time;
use std::collections::HashMap;
use std::fs;
use std::io::Error;
use std::io::ErrorKind;
use std::mem;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

struct IndexEntry {
    offset: u64,
    file_id: u32,
}

#[derive(Clone)]
pub struct DbDropGuard {
    db: DB,
}

impl DbDropGuard {
    pub fn new(options: Options) -> DbDropGuard {
        DbDropGuard {
            db: DB::open(options).unwrap(),
        }
    }
    pub fn db(&self) -> DB {
        self.db.clone()
    }
}

#[derive(Clone)]
pub struct DB {
    active_level: Arc<RwLock<ActiveUnit>>,

    levels: Arc<RwLock<Levels>>,
    //db config
    options: Options,
}

struct Levels {
    archived_level: Vec<ArchivedUnit>,
    merged_level: Vec<MergeUnit>,
}

impl Levels {
    fn new(file_size: u64, dir_path: String) -> IoResult<Levels> {
        let levels = Levels {
            archived_level: Vec::new(),
            merged_level: Vec::new(),
        };
        Ok(levels)
    }

    fn get(&self, key: &[u8]) -> IoResult<Entry> {
        for x in self.archived_level.iter().rev() {
            match x.get(key) {
                Ok(v) => return Ok(v),
                Err(_) => continue,
            };
        }
        Err(Error::from(ErrorKind::Interrupted))
    }
}

struct ActiveUnit {
    //active file
    active_file: DataFile,
    //archived files
    archived_files: HashMap<u32, DataFile>,
    //memory index message
    indexes: HashMap<Vec<u8>, IndexEntry>,
    //every one file size,
    file_size: usize,
    //data dir path
    data_dir: String,
    //froze archived files ,can't motify data
    froze_archived_files: Option<ArchivedUnit>,

    sender: Mutex<mpsc::Sender<i32>>,

    archievd_limit_num: u32,
}

impl ActiveUnit {
    fn new(
        file_size: u64,
        data_dir: String,
        sender: mpsc::Sender<i32>,
        archievd_limit_num: u32,
    ) -> IoResult<ActiveUnit> {
        let (active_file, archived_files) = build_data_file(&data_dir, file_size)?;
        let mut active_unit = ActiveUnit {
            active_file: active_file,
            archived_files: archived_files,
            indexes: HashMap::new(),
            file_size: file_size as usize,
            data_dir: data_dir,
            froze_archived_files: None,
            sender: Mutex::new(sender),
            archievd_limit_num: archievd_limit_num,
        };
        active_unit.load_index();
        Ok(active_unit)
    }

    fn load_index(&mut self) {
        let mut id_vec = self
            .archived_files
            .iter_mut()
            .map(|(_id, _)| _id.clone())
            .collect::<Vec<u32>>();
        id_vec.sort();
        let mut file_id;
        for id in id_vec.iter() {
            let data_file = self.archived_files.get_mut(id).unwrap();
            file_id = data_file.file_id;
            println!("file_id is {}", file_id);
            let mut iter = data_file.iterator();
            let mut offset: usize = 0;
            while offset + datafile::ENTRY_HEADER_SIZE < self.file_size {
                let e = iter.next();
                let entry = match e {
                    Ok(entry) => entry,
                    Err(_) => break,
                };
                self.indexes.insert(
                    entry.key.clone(),
                    IndexEntry {
                        offset: offset as u64,
                        file_id: file_id,
                    },
                );
                offset += entry.size();
            }
        }

        file_id = self.active_file.file_id;
        println!("active_file_id is {}", file_id);
        let mut iter = self.active_file.iterator();
        let mut offset: usize = 0;
        while offset + datafile::ENTRY_HEADER_SIZE < self.file_size {
            let e = iter.next();
            let entry = match e {
                Ok(entry) => entry,
                Err(_) => break,
            };
            self.indexes.insert(
                entry.key.clone(),
                IndexEntry {
                    offset: offset as u64,
                    file_id: file_id,
                },
            );
            offset += entry.size();
        }
        self.active_file.offset = offset;
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> IoResult<()> {
        let e = Entry {
            timestamp: time::get_time_unix_nano() as u64,
            key: key.to_vec(),
            value: value.to_vec(),
            crc: 0,
        };
        let index_entry = self.store(&e)?;
        self.indexes.insert(e.key, index_entry);
        Ok(())
    }

    fn get_from_active(&self, key: &[u8]) -> IoResult<Entry> {
        let index_entry = self
            .indexes
            .get(&key.to_vec())
            .ok_or(Error::from(ErrorKind::Interrupted))?;
        if index_entry.file_id == self.active_file.file_id {
            println!(
                "active_file.file_id={},offset is {}",
                index_entry.file_id, index_entry.offset
            );
            return self.active_file.get(index_entry.offset);
        }
        let file = self
            .archived_files
            .get(&index_entry.file_id)
            .ok_or(Error::from(ErrorKind::Interrupted))?;
        println!(
            "active_file.file_id={},offset is {}",
            index_entry.file_id, index_entry.offset
        );
        file.get(index_entry.offset)
    }

    fn get_from_froze(&self, key: &[u8]) -> IoResult<Entry> {}

    fn get(&self, key: &[u8]) -> IoResult<Entry> {
        self.get_from_active(key)
    }

    fn store(&mut self, e: &Entry) -> IoResult<IndexEntry> {
        let sz = e.size();
        let active_file_id: u32;
        {
            if self.active_file.offset + sz < self.file_size {
                let offset = self.active_file.put(e)?;
                println!(
                    "file_id is {}, offset is {}",
                    self.active_file.file_id, offset
                );
                return Ok(IndexEntry {
                    offset: offset,
                    file_id: self.active_file.file_id,
                });
            }
            self.active_file.sync()?;
            active_file_id = self.active_file.file_id;
        }
        let old_active_file_id = active_file_id;
        let active_file_id = old_active_file_id + 1;
        let mut new_active_data_file = DataFile::new(
            &self.data_dir,
            self.file_size as u64,
            active_file_id,
            DataType::String,
        )?;

        let offset = new_active_data_file.put(e)?;
        println!("file_id is {}, offset is {}", active_file_id, offset);
        let old_active_data_file = mem::replace(&mut self.active_file, new_active_data_file);
        self.archived_files
            .insert(old_active_file_id, old_active_data_file);
        if self.archived_files.len() >= self.archievd_limit_num as usize {
            self.froze_archived_files = Some(self.to_archived_unit());
            let sender = self.sender.lock().unwrap();
            sender.send(1);
        }

        Ok(IndexEntry {
            offset: offset,
            file_id: active_file_id,
        })
    }

    fn to_archived_unit(&mut self) -> ArchivedUnit {
        let new_archived_files: HashMap<u32, DataFile> = HashMap::new();
        let new_indexes: HashMap<Vec<u8>, IndexEntry> = HashMap::new();
        let old_archived_files = mem::replace(&mut self.archived_files, new_archived_files);
        let old_indexes = mem::replace(&mut self.indexes, new_indexes);
        let archived_unit = ArchivedUnit {
            archived_files: old_archived_files,
            indexes: old_indexes,
        };
        archived_unit
    }
}

struct ArchivedUnit {
    archived_files: HashMap<u32, DataFile>,
    //memory index message
    indexes: HashMap<Vec<u8>, IndexEntry>,
}

impl ArchivedUnit {
    fn get(&self, key: &[u8]) -> IoResult<Entry> {
        let index_entry = self
            .indexes
            .get(&key.to_vec())
            .ok_or(Error::from(ErrorKind::Interrupted))?;
        let file = self
            .archived_files
            .get(&index_entry.file_id)
            .ok_or(Error::from(ErrorKind::Interrupted))?;
        file.get(index_entry.offset)
    }
}

struct MergeUnit {
    archived_files: DataFile,
    indexes: HashMap<Vec<u8>, IndexEntry>,
}

impl MergeUnit {
    fn get() {}
}

struct SSTableUnit {}

impl DB {
    pub fn open(options: Options) -> IoResult<DB> {
        let dir_path = options
            .data_dir
            .to_str()
            .ok_or(Error::from(ErrorKind::Interrupted))?;
        fs::create_dir_all(dir_path).map_err(|err| Error::new(ErrorKind::Interrupted, err))?;
        let data_dir = options
            .data_dir
            .to_str()
            .ok_or(Error::from(ErrorKind::Interrupted))?;
        let (tx, rx): (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel();
        let db = DB {
            active_level: Arc::new(RwLock::new(ActiveUnit::new(
                options.file_size,
                dir_path.to_owned(),
                tx,
                options.archievd_limit_num,
            )?)),
            levels: Arc::new(RwLock::new(Levels::new(
                options.file_size,
                data_dir.to_owned(),
            )?)),
            options: options,
        };
        let l = db.levels.clone();
        let a = db.active_level.clone();
        thread::spawn(move || loop {
            let received = rx.recv().unwrap();
            println!("merge starting");
            //merge active to archievd
            let merge_unit: MergeUnit;
            {
                let active = a.try_read().unwrap();

                // let old_froze_archived_files = mem::replace(&mut active.froze_archived_files, None);
                // match old_froze_archived_files {
                //     Some(v) => {
                //         let mut levels = l.write().unwrap();
                //         levels.archived_level.push(v);
                //     }
                //     None => continue,
                // };
            }
            {}
        });
        Ok(db)
    }
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> IoResult<()> {
        let mut active = self.active_level.write().unwrap();
        active.put(key, value)
    }

    pub fn get(&self, key: &[u8]) -> IoResult<Entry> {
        {
            let active = self.active_level.read().unwrap();
            match active.get(key) {
                Ok(v) => return Ok(v),
                Err(_) => {}
            }
        }
        {
            let levels = self.levels.read().unwrap();
            levels.get(key)
        }
    }
}

fn build_data_file(dir_path: &str, size: u64) -> IoResult<(DataFile, HashMap<u32, DataFile>)> {
    let dir = fs::read_dir(dir_path)?;

    let names = dir
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                e.path().file_name().and_then(|n| {
                    n.to_str().and_then(|s| {
                        if s.contains(".a") {
                            return Some(String::from(s));
                        }
                        None
                    })
                })
            })
        })
        .collect::<Vec<String>>();
    let mut files_map: HashMap<u32, DataFile> = HashMap::new();
    if names.len() == 0 {
        let active_file = DataFile::new(dir_path, size, 0, DataType::String)?;
        return Ok((active_file, files_map));
    }
    let mut files = names
        .iter()
        .filter_map(|n| {
            let split_name: Vec<&str> = n.split(".").collect();
            split_name[0].parse::<u32>().ok()
        })
        .collect::<Vec<u32>>();

    let active_file = DataFile::new(dir_path, size, files[files.len() - 1], DataType::String)?;
    for i in 0..files.len() - 1 {
        files_map.insert(
            files[i],
            DataFile::new(dir_path, size, files[i], DataType::String)?,
        );
    }

    Ok((active_file, files_map))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_put() {
        let c = Options::default();
        let mut d = DB::open(c).unwrap();
        d.put("a".as_bytes(), "aaabbbccccccfffffffeeee".as_bytes())
            .unwrap();
        let value = d.get("a".as_bytes()).unwrap().value;
        println!("{:?}", String::from_utf8(value).unwrap());
    }
    #[test]
    fn test_read() {
        let c = Options::default();
        let mut d = DB::open(c).unwrap();
        let value = d.get("a".as_bytes()).unwrap().value;
        println!("{:?}", String::from_utf8(value).unwrap());
    }
}
