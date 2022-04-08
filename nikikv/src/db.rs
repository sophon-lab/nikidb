use crate::bucket::IBucket;
use crate::error::{NKError, NKResult};
use crate::page::{Meta, Page, PageFlag, Pgid};
use crate::{magic, version};
use page_size;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::prelude::FileExt;

const MAX_MAP_SIZE: u64 = 0x0FFF_FFFF; //256TB

const MAX_MMAP_STEP: u64 = 1 << 30;

fn get_page_size() -> u32 {
    page_size::get() as u32
}

pub struct DB {
    file: File,
    page_size: u32,
    mmap: Option<memmap::Mmap>,
}

pub struct Options {
    no_grow_sync: bool,

    read_only: bool,

    mmap_flags: u32,

    initial_mmap_size: u64,
}

pub static DEFAULT_OPTIONS: Options = Options {
    no_grow_sync: false,
    read_only: false,
    mmap_flags: 0,
    initial_mmap_size: 0,
};

impl DB {
    pub fn open(db_path: &str, options: Options) -> NKResult<DB> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(db_path)
            .map_err(|e| NKError::DBOpenFail(e))?;
        let size = f.metadata().map_err(|e| NKError::DBOpenFail(e))?.len();
        let mut db = Self::new(f);
        if size == 0 {
            db.init()?;
        } else {
            let mut buf = vec![0; 0x1000];
            db.file
                .read_at(&mut buf, 0)
                .map_err(|_e| ("can't read to file", _e))?;
            let m = db.page_in_buffer(&buf, 0).meta();
            m.validate()?;
            db.page_size = m.page_size;
            println!("read:checksum {}", m.checksum);
        }
        db.mmap(options.initial_mmap_size)?;
        Ok(db)
    }

    fn new(file: File) -> DB {
        Self {
            file: file,
            page_size: 0,
            mmap: None,
        }
    }

    fn init(&mut self) -> NKResult<()> {
        self.page_size = get_page_size();
        let mut buf: Vec<u8> = vec![0; 4 * self.page_size as usize];
        for i in 0..2 {
            let p = self.page_in_buffer_mut(&mut buf, i);
            p.id = i as Pgid;
            p.flags = PageFlag::MetaPageFlag;

            let m = p.meta_mut();
            m.magic = magic;
            m.version = version;
            m.page_size = self.page_size;
            m.freelist = 2;
            m.root = IBucket::new(3);
            m.pgid = 4;
            m.txid = 1;
            m.checksum = m.sum64();
        }

        // write an empty freelist at page 3
        let mut p = self.page_in_buffer_mut(&mut buf, 2);
        p.id = 2;
        p.flags = PageFlag::FreeListPageFlag;
        p.count = 0;

        p = self.page_in_buffer_mut(&mut buf, 3);
        p.id = 3;
        p.flags = PageFlag::LeafPageFlag;
        p.count = 0;

        self.write_at(&mut buf, 0)?;
        self.sync()?;

        Ok(())
    }

    fn write_at(&mut self, buf: &mut [u8], pos: u64) -> NKResult<()> {
        self.file
            .write_at(buf, pos)
            .map_err(|_e| ("can't write to file", _e))?;
        Ok(())
    }

    fn sync(&mut self) -> NKResult<()> {
        self.file.flush().map_err(|_e| ("can't flush file", _e))?;
        Ok(())
    }

    fn page_in_buffer_mut<'a>(&mut self, buf: &'a mut [u8], id: u32) -> &'a mut Page {
        Page::from_buf_mut(&mut buf[(id * self.page_size) as usize..])
    }

    fn page_in_buffer<'a>(&self, buf: &'a [u8], id: u32) -> &'a Page {
        Page::from_buf(&buf[(id * self.page_size) as usize..])
    }

    fn mmap_size(&self, mut size: u64) -> NKResult<u64> {
        for i in 15..=30 {
            if size <= 1 << i {
                return Ok(1 << i);
            }
        }
        if size > MAX_MAP_SIZE {
            return Err(NKError::Unexpected("mmap too large".to_string()));
        }
        let remainder = size % MAX_MMAP_STEP;
        if remainder > 0 {
            size += MAX_MAP_SIZE - remainder;
        };
        let page_size = self.page_size as u64;
        if (size % page_size) != 0 {
            size = ((size / page_size) + 1) * page_size;
        };
        // If we've exceeded the max size then only grow up to the max size.
        if size > MAX_MAP_SIZE {
            size = MAX_MAP_SIZE
        };
        Ok(size)
    }

    fn munmap() {}

    fn mmap(&mut self, mut min_size: u64) -> NKResult<()> {
        let mut mmap_opts = memmap::MmapOptions::new();

        let mut size = self
            .file
            .metadata()
            .map_err(|e| NKError::DBOpenFail(e))?
            .len();
        if size < min_size {
            size = min_size;
        }
        min_size = self.mmap_size(size)?;
        // let  munmap =
        let nmmap = unsafe {
            mmap_opts
                .offset(0)
                .len(min_size as usize)
                .map(&self.file)
                .map_err(|e| format!("mmap failed: {}", e))?
        };
        let meta0 = self.page_in_buffer(&nmmap, 0).meta();
        let meta1 = self.page_in_buffer(&nmmap, 1).meta();
        meta0.validate()?;
        meta1.validate()?;
        self.mmap = Some(nmmap);
        Ok(())
    }

    pub fn update() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_db_open() {
        DB::open("./test.db", DEFAULT_OPTIONS).unwrap();
    }
}
