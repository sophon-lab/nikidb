use std::collections::HashMap;

use crate::cursor::Cursor;
use crate::error::{NKError, NKResult};
use crate::page::{BucketLeafFlag, Node, Page, Pgid};
use crate::tx::TxImpl;
use std::sync::{Arc, Weak};

pub(crate) struct Bucket {
    pub(crate) ibucket: IBucket,
    nodes: HashMap<Pgid, Node>, //tx: Tx,
    pub(crate) weak_tx: Weak<TxImpl>,
    rootNode: Node,
}

#[derive(Clone)]
pub(crate) enum PageNode {
    Page(*const Page),
    Node(Node),
}

impl From<Node> for PageNode {
    fn from(n: Node) -> Self {
        PageNode::Node(n)
    }
}

impl Bucket {
    pub(crate) fn new(root: Pgid, tx: Weak<TxImpl>) -> Bucket {
        Self {
            ibucket: IBucket {
                root: root,
                sequence: 0,
            },
            nodes: HashMap::new(),
            weak_tx: tx,
        }
    }

    pub(crate) fn create_bucket(&mut self, key: &[u8]) -> NKResult<Bucket> {
        let mut c = self.cursor();
        let item = c.seek()?;
        if item.key().eq(key) {
            if item.flags() & BucketLeafFlag != 0 {
                return NKError::ErrBucketExists(String::from_utf8_lossy(key));
            }
            return NKError::ErrIncompatibleValue;
        }

        //
        let bucket = Bucket::new(0, self.weak_tx.clone());
        Ok(bucket)
    }

    fn cursor(&mut self) -> Cursor {
        Cursor::new(self)
        //    let item =
        // Cursor { bucket: self }
    }

    pub(crate) fn put(key: &[u8], value: &[u8]) {}

    pub(crate) fn get(key: &[u8]) {}

    pub(crate) fn page_node(&self, id: Pgid) -> NKResult<PageNode> {
        if let Some(node) = self.nodes.get(&id) {
            return Ok(PageNode::Node(node.clone()));
        }
        let page = self.tx().unwrap().db().page(id);
        Ok(PageNode::Page(page))
    }

    pub(crate) fn tx(&self) -> Option<Arc<TxImpl>> {
        self.weak_tx.upgrade()
    }

    pub(crate) fn value() {}
}

pub(crate) struct IBucket {
    pub(crate) root: Pgid,
    sequence: u64,
}

impl IBucket {
    pub(crate) fn new(root: Pgid) -> IBucket {
        Self {
            root: root,
            sequence: 0,
        }
    }
}
