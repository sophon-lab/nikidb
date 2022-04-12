use crate::bucket::Bucket;
use crate::page::Page;

pub struct Cursor<'a> {
    bucket: &'a Bucket,
}

impl<'a> Cursor<'a> {
    fn first(&mut self) {}

    fn last(&mut self) {}

    fn next(&mut self) {}

    fn prev(&mut self) {}

    fn delete(&mut self) {}

    pub fn seek(&mut self) {}

    fn _seek(&mut self) {}
}

struct ElemRef {
    page: Page,
}
