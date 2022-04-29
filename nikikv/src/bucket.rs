use crate::page::Pgid;
use crate::tx::Tx;

pub struct Bucket {
    bucket: IBucket,
    //tx: Tx,
}

impl Bucket {
    pub fn new(root: Pgid) {}

    pub fn create_bucket(&mut self) {
        let mut c = self.cursor();
    }

    fn cursor(&mut self) -> Cursor {}

    pub fn put(key: &[u8], value: &[u8]) {}

    pub fn get(key: &[u8]) {}

    pub fn page_node(id: Pgid) -> () {}

    pub fn value() -> &[u8] {}
}

pub struct IBucket {
    root: Pgid,
    sequence: u64,
}

impl IBucket {
    pub fn new(root: Pgid) -> IBucket {
        Self {
            root: root,
            sequence: 0,
        }
    }
}
