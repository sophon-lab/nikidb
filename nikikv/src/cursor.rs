use std::borrow::BorrowMut;

use crate::bucket::{Bucket, PageNode};
use crate::error::NKResult;
use crate::page::{Node, Page, PageFlag, Pgid};

pub(crate) struct Cursor<'a> {
    pub(crate) bucket: &'a Bucket,
    stack: Vec<ElemRef>,
}

#[derive(Clone)]
pub(crate) struct ElemRef {
    page_node: PageNode,
    index: i32,
}

impl ElemRef {
    fn is_leaf(&self) -> bool {
        match &self.page_node {
            PageNode::Node(n) => n.is_leaf,
            PageNode::Page(p) => unsafe { (*(*p)).flags == PageFlag::LeafPageFlag },
        }
    }
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bucket: &'a Bucket) -> Cursor<'a> {
        Self {
            bucket: bucket,
            stack: Vec::new(),
        }
    }

    fn first(&mut self) {}

    fn last(&mut self) {}

    fn next(&mut self) {}

    fn prev(&mut self) {}

    fn delete(&mut self) {}

    pub fn seek(&mut self) {}

    //查询
    fn search(&mut self, key: &[u8], id: Pgid) -> NKResult<()> {
        let page_node = self.bucket.page_node(id)?;
        let elem_ref = ElemRef {
            page_node: page_node,
            index: 0,
        };
        self.stack.push(elem_ref.clone());
        if elem_ref.is_leaf() {
            self.nsearch()
        }
        match elem_ref.page_node {
            PageNode::Node(n) => self.search_node(key, &n),
            PageNode::Page(p) => self.search_page(key, unsafe { &*p }),
        }
        Ok(())
    }

    fn nsearch(&self) {}

    fn search_page(&self, key: &[u8], p: &Page) {
        let inodes = p.branch_page_elements();
        //inodes.binary_search_by_key(b, f)
    }

    fn search_node(&self, key: &[u8], p: &Node) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sort_search() {
        let s = [0, 1, 1, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55];

        println!("{:?}", s.binary_search(&14));
    }
}
