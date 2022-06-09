use crate::bucket::{Bucket, IBucket};
use crate::page::{
    BranchPageElementSize, BranchPageFlag, BucketLeafFlag, FreeListPageFlag, LeafPageElementSize,
    LeafPageFlag, MetaPageFlag, Page, Pgid, MIN_KEY_PERPAGE,
};
use crate::tx::{Tx, TxImpl};
use crate::{error::NKError, error::NKResult};
use crate::{magic, version};
use fnv::FnvHasher;
use memoffset::offset_of;
use std::borrow::BorrowMut;
use std::cell::{Ref, RefCell, RefMut};
use std::hash::Hasher;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ops::Sub;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::rc::Weak;
use std::sync::{Arc, Weak as ArcWeak};

#[derive(Clone)]
pub(crate) struct Node(pub(crate) Rc<RefCell<NodeImpl>>);

#[derive(Clone)]
pub(crate) struct NodeImpl {
    // pub(crate) bucket: *mut Bucket,
    pub(crate) is_leaf: bool,
    pub(crate) inodes: Vec<INode>,
    pub(crate) parent: Option<Weak<RefCell<NodeImpl>>>,
    unbalanced: bool,
    spilled: bool,
    pub(crate) pgid: Pgid,
    pub(crate) children: Vec<Node>,
    key: Option<Vec<u8>>,
}

impl NodeImpl {
    pub(crate) fn new() -> NodeImpl {
        Self {
            //  bucket: bucket,
            is_leaf: false,
            inodes: Vec::new(),
            parent: None,
            unbalanced: false,
            spilled: false,
            pgid: 0,
            children: Vec::new(),
            key: None,
        }
    }

    pub fn leaf(mut self, is_leaf: bool) -> NodeImpl {
        self.is_leaf = is_leaf;
        self
    }

    pub fn parent(mut self, parent: Weak<RefCell<NodeImpl>>) -> NodeImpl {
        self.parent = Some(parent);
        self
    }

    pub(crate) fn build(self) -> Node {
        Node(Rc::new(RefCell::new(self)))
    }
}

impl Node {
    pub(crate) fn node_mut(&mut self) -> RefMut<'_, NodeImpl> {
        (*(self.0)).borrow_mut()
    }

    pub(crate) fn node(&self) -> Ref<'_, NodeImpl> {
        self.0.borrow()
    }

    pub(crate) fn child_at(
        &mut self,
        bucket: &mut Bucket,
        index: usize,
        parent: Option<Weak<RefCell<NodeImpl>>>,
    ) -> Node {
        if self.node().is_leaf {
            panic!("invalid childAt{} on a leaf node", index);
        }
        bucket.node(self.node().inodes[index].pgid, parent)
    }

    pub(crate) fn size(&self) -> usize {
        let mut sz = Page::header_size();
        let elsz = self.page_element_size();
        let a = self.node();
        for i in 0..a.inodes.len() {
            let item = a.inodes.get(i).unwrap();
            sz += elsz + item.key.len() + item.value.len();
        }
        sz
    }

    fn page_element_size(&self) -> usize {
        if self.node().is_leaf {
            return LeafPageElementSize;
        }
        BranchPageElementSize
    }

    pub(crate) fn read(&mut self, p: &Page) {
        self.node_mut().pgid = p.id;
        self.node_mut().is_leaf = (p.flags & LeafPageFlag) != 0;
        let count = p.count as usize;
        self.node_mut().inodes = Vec::with_capacity(count);
        for i in 0..count {
            let mut inode = INode::new();
            if self.node().is_leaf {
                let elem = p.leaf_page_element(i);
                inode.flags = elem.flags;
                inode.key = elem.key().to_vec();
                inode.value = elem.value().to_vec();
            } else {
                let elem = p.branch_page_element(i);
                inode.pgid = elem.pgid;
                inode.key = elem.key().to_vec();
            }
            assert!(inode.key.len() > 0, "read: zero-length inode key");
        }

        if self.node().inodes.len() > 0 {
            let key = { self.node().inodes.first().unwrap().key.clone() };
            self.node_mut().key = Some(key);
        } else {
            self.node_mut().key = None
        }
    }

    pub(crate) fn put(
        &mut self,
        old_key: &[u8],
        new_key: &[u8],
        value: &[u8],
        pgid: Pgid,
        flags: u32,
    ) {
        // if pgid > bucket.tx().unwrap().meta.borrow().pgid {
        //     panic!(
        //         "pgid {} above high water mark {}",
        //         pgid,
        //         bucket.tx().unwrap().meta.borrow().pgid,
        //     )
        // } else
        if old_key.len() <= 0 {
            panic!("put: zero-length old key")
        } else if new_key.len() <= 0 {
            panic!("put: zero-length new key")
        }
        let (exact, index) = {
            match self
                .node()
                .inodes
                .binary_search_by(|inode| inode.key.as_slice().cmp(old_key))
            {
                Ok(v) => (true, v),
                Err(e) => (false, e),
            }
        };
        let mut n1 = self.node_mut();
        if !exact {
            n1.inodes.insert(index, INode::new());
        }
        let inode = n1.inodes.get_mut(index).unwrap();
        inode.flags = flags;
        inode.key = new_key.to_vec();
        inode.value = value.to_vec();
        inode.pgid = pgid;
        assert!(inode.key.len() > 0, "put: zero-length inode key")
    }

    pub(crate) fn write(&self, p: &mut Page) {
        if self.node().is_leaf {
            p.flags = LeafPageFlag;
        } else {
            p.flags = BranchPageFlag;
        }
        if self.node().inodes.len() > 0xFFF {
            panic!(
                "inode overflow: {} (pgid={})",
                self.node().inodes.len(),
                p.id
            );
        }
        p.count = self.node().inodes.len() as u16;
        if p.count == 0 {
            return;
        }

        let mut buf_ptr = unsafe {
            p.data_ptr_mut()
                .add(self.page_element_size() * self.node().inodes.len())
        };

        for (i, item) in self.node().inodes.iter().enumerate() {
            assert!(item.key.len() > 0, "write: zero-length inode key");
            if self.node().is_leaf {
                let elem = p.leaf_page_element_mut(i);
                elem.pos = unsafe { buf_ptr.sub(elem.as_ptr() as usize) } as u32;
                elem.flags = item.flags as u32;
                elem.ksize = item.key.len() as u32;
                elem.vsize = item.value.len() as u32;
            } else {
                let elem = p.branch_page_element_mut(i);
                elem.pos = unsafe { buf_ptr.sub(elem.as_ptr() as usize) } as u32;
                elem.ksize = unsafe { buf_ptr.sub(elem.as_ptr() as usize) } as u32;
                elem.pgid = item.pgid;
                assert!(elem.pgid != p.id, "write: circular dependency occurred");
            }
            let (klen, vlen) = (item.key.len(), item.value.len());
            unsafe {
                std::ptr::copy_nonoverlapping(item.key.as_ptr(), buf_ptr, klen);
                buf_ptr = buf_ptr.add(klen);
                std::ptr::copy_nonoverlapping(item.value.as_ptr(), buf_ptr, vlen);
                buf_ptr = buf_ptr.add(vlen);
            }
        }
    }

    pub(crate) fn root(&self, node: Node) -> Node {
        if let Some(parent_node) = &self.node().parent {
            let p = parent_node.upgrade().map(Node).unwrap();
            p.root(p.clone())
        } else {
            node
        }
    }

    //删除元素 重平衡
    fn rebalance(&mut self) {}

    //添加元素 分裂
    fn split(&mut self, page_size: u32, fill_percent: f64) -> Vec<Node> {
        let nodes: Vec<Node> = Vec::new();

        nodes
    }

    fn split_two(&mut self, page_size: u32, fill_percent: f64) {
        //-> (Node, Node)
        if self.node().inodes.len() <= MIN_KEY_PERPAGE * 2 {}
    }

    fn node_less_than(&mut self) {
        let mut sz = Page::header_size();
        let elsz = self.page_element_size();
        let a = self.node();
        for i in 0..a.inodes.len() {
            let item = a.inodes.get(i).unwrap();
            sz += elsz + item.key.len() + item.value.len();
        }
        sz
    }

    //node spill
    pub(crate) fn spill(&mut self, atx: Arc<TxImpl>, bucket: &Bucket) -> NKResult<()> {
        if self.node().spilled {
            return Ok(());
        }

        self.node_mut()
            .children
            .sort_by(|a, b| (*a).node().inodes[0].key.cmp(&(*b).node().inodes[0].key));
        for mut child in self.node_mut().children.clone() {
            child.spill(atx.clone(), bucket)?;
        }

        self.node_mut().children.clear();
        let tx = atx.clone();
        let db = tx.db();

        let nodes = self.split(db.get_page_size(), bucket.fill_percent);

        if self.node().pgid > 0 {
            db.freelist
                .try_write()
                .unwrap()
                .free(tx.meta.borrow().txid, unsafe {
                    &*db.page(self.node().pgid)
                });
            self.node_mut().pgid = 0;
        }

        let mut p = db.allocate(self.size() / db.get_page_size() as usize + 1)?;
        let page = p.to_page_mut();
        if page.id >= tx.meta.borrow().pgid {
            panic!(
                "pgid {} above high water mark{}",
                page.id,
                tx.meta.borrow().pgid
            );
        }
        self.node_mut().pgid = page.id;
        self.write(page);
        tx.pages.borrow_mut().insert(self.node().pgid, p);
        self.node_mut().spilled = true;

        if let Some(parent) = &self.node().parent {
            let mut parent_node = parent.upgrade().map(Node).unwrap();
            if let Some(key) = &self.node().key {
                parent_node.put(key, key, &vec![], self.node().pgid, 0);
            } else {
                let key = {
                    let n1 = self.node();
                    let inode = n1.inodes.first().unwrap();
                    parent_node.put(&inode.key, &inode.key, &vec![], self.node().pgid, 0);
                    inode.key.clone()
                };
            }
        }

        // self.node_mut().key = Some(key);
        // self.node_mut().children.clear();
        // return parent_node.spill(atx.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct INode {
    pub(crate) flags: u32,
    pub(crate) pgid: Pgid,
    pub(crate) key: Vec<u8>,
    pub(crate) value: Vec<u8>,
}

impl INode {
    fn new() -> INode {
        Self {
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::null_mut;

    use super::*;
    #[test]
    fn test_node_new() {
        let n = NodeImpl::new().build();
    }
}
