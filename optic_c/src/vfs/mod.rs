use crate::arena::{Arena, CAstNode, NodeOffset, NodeFlags};
use crate::analysis::alias::AliasAnalysis;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use fuser::{Filesystem, FileAttr, FileType, FileHandle, OpenFlags, FopenFlags, Errno};
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

const FUSE_ROOT_ID: u64 = 1;

pub struct Vfs {
    arena: Arc<Arena>,
    analysis: Arc<AliasAnalysis>,
    mount_path: String,
    file_nodes: RwLock<HashMap<u64, VfsNode>>,
    next_inode: Mutex<u64>,
}

#[derive(Debug, Clone)]
struct VfsNode {
    name: String,
    inode: u64,
    file_type: FileType,
    content: Option<Vec<u8>>,
    children: Vec<u64>,
    parent: u64,
}

impl Vfs {
    pub fn new(arena: Arc<Arena>, analysis: Arc<AliasAnalysis>, mount_path: &str) -> Self {
        let vfs = Self {
            arena,
            analysis,
            mount_path: mount_path.to_string(),
            file_nodes: RwLock::new(HashMap::new()),
            next_inode: Mutex::new(FUSE_ROOT_ID + 1),
        };
        vfs.init_root();
        vfs
    }

    fn init_root(&self) {
        let root = VfsNode {
            name: String::new(),
            inode: FUSE_ROOT_ID,
            file_type: FileType::Directory,
            content: None,
            children: Vec::new(),
            parent: FUSE_ROOT_ID,
        };
        self.file_nodes.write().unwrap().insert(FUSE_ROOT_ID, root);
    }

    fn next_inode(&self) -> u64 {
        let mut next = self.next_inode.lock().unwrap();
        let inode = *next;
        *next += 1;
        inode
    }

    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }

    pub fn reconstruct_from_arena(&self) {
        self.init_root();
        
        let src_dir_inode = self.next_inode();
        self.add_dir(FUSE_ROOT_ID, ".optic", src_dir_inode);
        
        let vfs_src_inode = self.next_inode();
        self.add_dir(src_dir_inode, "vfs", vfs_src_inode);
        
        let src_inode = self.next_inode();
        self.add_dir(vfs_src_inode, "src", src_inode);
        
        if let Some(root_node) = self.find_root_node() {
            self.reconstruct_tree(root_node, src_inode);
        }
    }

    fn add_dir(&self, parent: u64, name: &str, inode: u64) {
        if let Some(parent_node) = self.file_nodes.write().unwrap().get_mut(&parent) {
            parent_node.children.push(inode);
        }
        
        let node = VfsNode {
            name: name.to_string(),
            inode,
            file_type: FileType::Directory,
            content: None,
            children: Vec::new(),
            parent,
        };
        self.file_nodes.write().unwrap().insert(inode, node);
    }

    fn add_file(&self, parent: u64, name: &str, content: Vec<u8>) -> u64 {
        let inode = self.next_inode();
        
        if let Some(parent_node) = self.file_nodes.write().unwrap().get_mut(&parent) {
            parent_node.children.push(inode);
        }
        
        let node = VfsNode {
            name: name.to_string(),
            inode,
            file_type: FileType::RegularFile,
            content: Some(content),
            children: Vec::new(),
            parent,
        };
        self.file_nodes.write().unwrap().insert(inode, node);
        inode
    }

    fn find_root_node(&self) -> Option<NodeOffset> {
        let mut cursor = NodeOffset(1);
        while let Some(node) = self.arena.get(cursor) {
            if node.kind == 0 && node.parent == NodeOffset::NULL {
                return Some(cursor);
            }
            cursor = NodeOffset(cursor.0 + 1);
            if cursor.0 >= self.arena.capacity() {
                break;
            }
        }
        None
    }

    fn reconstruct_tree(&self, root: NodeOffset, parent_inode: u64) {
        let mut queue = vec![(root, parent_inode)];
        
        while let Some((offset, parent)) = queue.pop() {
            if let Some(node) = self.arena.get(offset) {
                if node.flags.contains(NodeFlags::IS_VALID) {
                    let node_kind = node.kind;
                    let node_data = node.data;
                    let file_name = self.get_node_name(node);
                    let first_child = node.first_child;
                    drop(node);
                    
                    let content = self.reconstruct_file_content(node_kind, node_data, first_child);
                    
                    let file_inode = self.add_file(parent, &file_name, content);
                    
                    let mut child = first_child;
                    while child != NodeOffset::NULL {
                        if let Some(child_node) = self.arena.get(child) {
                            queue.push((child, file_inode));
                            child = child_node.next_sibling;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }

    fn get_node_name(&self, node: &CAstNode) -> String {
        if node.data != 0 {
            if let Some(str_ptr) = self.arena.get_string(node.data) {
                return str_ptr.to_string();
            }
        }
        format!("node_{}.c", node.kind)
    }

    fn reconstruct_file_content(&self, kind: u16, data: u32, first_child: NodeOffset) -> Vec<u8> {
        let mut content = String::new();
        content.push_str("/* OPTIC RECONSTRUCTED FILE */\n");
        content.push_str(&format!("/* NodeKind: {}, DataOffset: {} */\n", kind, data));
        
        let mut child = first_child;
        while child != NodeOffset::NULL {
            if let Some(child_node) = self.arena.get(child) {
                content.push_str(&self.node_to_source(child_node));
                content.push('\n');
                child = child_node.next_sibling;
            } else {
                break;
            }
        }
        
        content.into_bytes()
    }

    fn node_to_source(&self, node: &CAstNode) -> String {
        let kind_name = self.kind_name(node.kind);
        format!("/* {} */", kind_name)
    }

    fn kind_name(&self, kind: u16) -> &'static str {
        match kind {
            1 => "FUNCTION_DECL",
            2 => "VAR_DECL",
            3 => "BINARY_OP",
            4 => "UNARY_OP",
            5 => "CALL_EXPR",
            6 => "IF_STMT",
            7 => "WHILE_STMT",
            8 => "RETURN_STMT",
            _ => "UNKNOWN",
        }
    }

    fn inject_error_comments(&self, file_inode: u64, content: Vec<u8>) -> Vec<u8> {
        let source = String::from_utf8_lossy(&content);
        let mut result = String::new();
        
        for line in source.lines() {
            if let Some(vulnerable_line) = self.check_vulnerability(line) {
                result.push_str("// [OPTIC ERROR] ");
                result.push_str(&vulnerable_line);
                result.push_str("\n");
            }
            result.push_str(line);
            result.push('\n');
        }
        
        result.into_bytes()
    }

    fn check_vulnerability(&self, line: &str) -> Option<String> {
        if self.analysis.is_vulnerable(line) {
            Some(line.to_string())
        } else {
            None
        }
    }

    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        
        if parts.len() < 4 || parts[0] != ".optic" || parts[1] != "vfs" || parts[2] != "src" {
            return None;
        }
        
        let file_name = parts[3];
        let mut current_inode = FUSE_ROOT_ID;
        
        for (i, part) in parts.iter().enumerate() {
            if i <= 2 {
                continue;
            }
            
            if let Some(node) = self.file_nodes.read().unwrap().get(&current_inode) {
                let child_inode = node.children.iter().find(|&&child_inode| {
                    self.file_nodes.read().unwrap().get(&child_inode)
                        .map(|n| n.name == *part)
                        .unwrap_or(false)
                });
                
                if let Some(&child) = child_inode {
                    current_inode = child;
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        
        if let Some(node) = self.file_nodes.read().unwrap().get(&current_inode) {
            if let Some(content) = &node.content {
                if self.analysis.has_vulnerabilities() {
                    return Some(self.inject_error_comments(current_inode, content.clone()));
                }
                return Some(content.clone());
            }
        }
        
        None
    }
}

impl Filesystem for Vfs {
    fn lookup(&self, _req: &fuser::Request, parent: fuser::INodeNo, name: &std::ffi::OsStr, reply: fuser::ReplyEntry) {
        let name_str = name.to_str().unwrap_or("");
        
        if let Some(parent_node) = self.file_nodes.read().unwrap().get(&parent.0) {
            if let Some(&child_inode) = parent_node.children.iter().find(|&&child_inode| {
                self.file_nodes.read().unwrap().get(&child_inode)
                    .map(|n| n.name == name_str)
                    .unwrap_or(false)
            }) {
                if let Some(node) = self.file_nodes.read().unwrap().get(&child_inode) {
                    let attr = FileAttr {
                        ino: fuser::INodeNo(node.inode),
                        size: node.content.as_ref().map(|c| c.len() as u64).unwrap_or(4096),
                        blocks: 1,
                        blksize: 512,
                        atime: SystemTime::UNIX_EPOCH,
                        mtime: SystemTime::UNIX_EPOCH,
                        ctime: SystemTime::UNIX_EPOCH,
                        crtime: SystemTime::UNIX_EPOCH,
                        kind: node.file_type,
                        perm: if node.file_type == FileType::Directory { 0o755 } else { 0o644 },
                        nlink: if node.file_type == FileType::Directory { 2 } else { 1 },
                        uid: 0,
                        gid: 0,
                        rdev: 0,
                        flags: 0,
                    };
                    reply.entry(&std::time::Duration::new(0, 0), &attr, fuser::Generation(0));
                    return;
                }
            }
        }
        reply.error(fuser::Errno::ENOENT);
    }

    fn getattr(&self, _req: &fuser::Request, ino: fuser::INodeNo, _fh: Option<FileHandle>, reply: fuser::ReplyAttr) {
        if let Some(node) = self.file_nodes.read().unwrap().get(&ino.0) {
            let attr = FileAttr {
                ino: fuser::INodeNo(node.inode),
                size: node.content.as_ref().map(|c| c.len() as u64).unwrap_or(4096),
                blocks: 1,
                blksize: 512,
                atime: SystemTime::UNIX_EPOCH,
                mtime: SystemTime::UNIX_EPOCH,
                ctime: SystemTime::UNIX_EPOCH,
                crtime: SystemTime::UNIX_EPOCH,
                kind: node.file_type,
                perm: if node.file_type == FileType::Directory { 0o755 } else { 0o644 },
                nlink: if node.file_type == FileType::Directory { 2 } else { 1 },
                uid: 0,
                gid: 0,
                rdev: 0,
                flags: 0,
            };
            reply.attr(&std::time::Duration::new(0, 0), &attr);
        } else {
            reply.error(fuser::Errno::ENOENT);
        }
    }

    fn readdir(&self, _req: &fuser::Request, ino: fuser::INodeNo, _fh: FileHandle, offset: u64, mut reply: fuser::ReplyDirectory) {
        if ino.0 == FUSE_ROOT_ID {
            let entries: Vec<(u64, FileType, &str)> = vec![
                (FUSE_ROOT_ID, FileType::Directory, "."),
                (FUSE_ROOT_ID, FileType::Directory, ".."),
            ];
            
            if let Some(node) = self.file_nodes.read().unwrap().get(&ino.0) {
                for (i, &child_inode) in node.children.iter().enumerate() {
                    if i as u64 >= offset {
                        if let Some(child) = self.file_nodes.read().unwrap().get(&child_inode) {
                            reply.add(fuser::INodeNo(child_inode), i as u64 + 2, child.file_type, &child.name);
                        }
                    }
                }
            }
            
            for (i, entry) in entries.iter().enumerate() {
                if i as u64 >= offset {
                    reply.add(fuser::INodeNo(entry.0), i as u64 + 1, entry.1, entry.2);
                }
            }
            reply.ok();
        } else if let Some(node) = self.file_nodes.read().unwrap().get(&ino.0) {
            if node.file_type == FileType::Directory {
                reply.add(fuser::INodeNo(ino.0), 0, FileType::Directory, ".");
                reply.add(fuser::INodeNo(node.parent), 1, FileType::Directory, "..");
                
                for (i, &child_inode) in node.children.iter().enumerate() {
                    if let Some(child) = self.file_nodes.read().unwrap().get(&child_inode) {
                        reply.add(fuser::INodeNo(child_inode), i as u64 + 2, child.file_type, &child.name);
                    }
                }
                reply.ok();
            } else {
                reply.error(fuser::Errno::ENOTDIR);
            }
        } else {
            reply.error(fuser::Errno::ENOENT);
        }
    }

    fn read(&self, _req: &fuser::Request, ino: fuser::INodeNo, _fh: FileHandle, offset: u64, size: u32, _flags: fuser::OpenFlags, _lock_owner: Option<fuser::LockOwner>, reply: fuser::ReplyData) {
        if let Some(node) = self.file_nodes.read().unwrap().get(&ino.0) {
            if let Some(content) = &node.content {
                let start = offset as usize;
                
                if start >= content.len() {
                    reply.error(fuser::Errno::EINVAL);
                    return;
                }
                
                let end = std::cmp::min(start + size as usize, content.len());
                let mut data = content[start..end].to_vec();
                
                if self.analysis.has_vulnerabilities() {
                    let modified = self.inject_error_comments(ino.0, data);
                    reply.data(&modified);
                } else {
                    reply.data(&data);
                }
            } else {
                reply.error(fuser::Errno::EISDIR);
            }
        } else {
            reply.error(fuser::Errno::ENOENT);
        }
    }

    fn opendir(&self, _req: &fuser::Request, ino: fuser::INodeNo, _flags: OpenFlags, reply: fuser::ReplyOpen) {
        if let Some(node) = self.file_nodes.read().unwrap().get(&ino.0) {
            if node.file_type == FileType::Directory {
                reply.opened(FileHandle(0), FopenFlags::empty());
            } else {
                reply.error(fuser::Errno::ENOTDIR);
            }
        } else {
            reply.error(fuser::Errno::ENOENT);
        }
    }

    fn releasedir(&self, _req: &fuser::Request, _ino: fuser::INodeNo, _fh: FileHandle, _flags: OpenFlags, reply: fuser::ReplyEmpty) {
        reply.ok();
    }

    fn open(&self, _req: &fuser::Request, ino: fuser::INodeNo, _flags: OpenFlags, reply: fuser::ReplyOpen) {
        if let Some(node) = self.file_nodes.read().unwrap().get(&ino.0) {
            if node.file_type == FileType::RegularFile {
                reply.opened(FileHandle(0), FopenFlags::empty());
            } else {
                reply.error(fuser::Errno::EISDIR);
            }
        } else {
            reply.error(fuser::Errno::ENOENT);
        }
    }

    fn release(&self, _req: &fuser::Request, _ino: fuser::INodeNo, _fh: FileHandle, _flags: OpenFlags, _lock_owner: Option<fuser::LockOwner>, _flush: bool, reply: fuser::ReplyEmpty) {
        reply.ok();
    }
}

pub trait ArenaAccess {
    fn capacity(&self) -> u32;
    fn get_string(&self, offset: u32) -> Option<&str>;
}

impl ArenaAccess for Arena {
    fn capacity(&self) -> u32 {
        self.allocated()
    }
    
    fn get_string(&self, offset: u32) -> Option<&str> {
        None
    }
}