use crate::{rand::RandomHandle, task::TaskHandle, time::TimeHandle};
use log::*;
use std::{
    collections::HashMap,
    io::{Error, ErrorKind, Result},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

pub struct FileSystemRuntime {
    handles: Mutex<HashMap<SocketAddr, FileSystemHandle>>,
    rand: RandomHandle,
    time: TimeHandle,
    task: TaskHandle,
}

impl FileSystemRuntime {
    pub(crate) fn new(rand: RandomHandle, time: TimeHandle, task: TaskHandle) -> Self {
        FileSystemRuntime {
            handles: Mutex::new(HashMap::new()),
            rand,
            time,
            task,
        }
    }

    pub fn handle(&self, addr: SocketAddr) -> FileSystemHandle {
        let mut handles = self.handles.lock().unwrap();
        handles
            .entry(addr)
            .or_insert_with(|| Arc::new(FileSystem::new(addr)))
            .clone()
    }

    /// Simulate a power failure. All data that does not reach the disk will be lost.
    pub fn power_fail(&self, _addr: SocketAddr) {
        todo!()
    }
}

pub type FileSystemHandle = Arc<FileSystem>;

pub struct FileSystem {
    addr: SocketAddr,
    fs: Mutex<HashMap<PathBuf, Arc<INode>>>,
}

impl FileSystem {
    fn new(addr: SocketAddr) -> Self {
        trace!("fs: new at {}", addr);
        FileSystem {
            addr,
            fs: Mutex::new(HashMap::new()),
        }
    }

    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File> {
        let path = path.as_ref();
        trace!("fs({}): open at {:?}", self.addr, path);
        let fs = self.fs.lock().unwrap();
        let inode = fs
            .get(path)
            .ok_or(Error::new(
                ErrorKind::NotFound,
                format!("file not found: {:?}", path),
            ))?
            .clone();
        Ok(File {
            inode,
            can_write: false,
        })
    }

    pub async fn create(&self, path: impl AsRef<Path>) -> Result<File> {
        let path = path.as_ref();
        trace!("fs({}): create at {:?}", self.addr, path);
        let mut fs = self.fs.lock().unwrap();
        let inode = fs
            .entry(path.into())
            .or_insert_with(|| Arc::new(INode::new(path)))
            .clone();
        Ok(File {
            inode,
            can_write: true,
        })
    }
}

struct INode {
    path: PathBuf,
    data: RwLock<Vec<u8>>,
}

impl INode {
    fn new(path: &Path) -> Self {
        INode {
            path: path.into(),
            data: RwLock::new(Vec::new()),
        }
    }
}

pub struct File {
    inode: Arc<INode>,
    can_write: bool,
}

impl File {
    pub async fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize> {
        trace!(
            "file({:?}): read_at: offset={}, len={}",
            self.inode.path,
            offset,
            buf.len()
        );
        let data = self.inode.data.read().unwrap();
        let end = data.len().min(offset as usize + buf.len());
        let len = end - offset as usize;
        buf[..len].copy_from_slice(&data[offset as usize..end]);
        // TODO: random delay
        Ok(len)
    }

    pub async fn write_all_at(&self, buf: &[u8], offset: u64) -> Result<()> {
        trace!(
            "file({:?}): write_all_at: offset={}, len={}",
            self.inode.path,
            offset,
            buf.len()
        );
        if !self.can_write {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "the file is read only",
            ));
        }
        let mut data = self.inode.data.write().unwrap();
        let end = data.len().min(offset as usize + buf.len());
        let len = end - offset as usize;
        data[offset as usize..end].copy_from_slice(&buf[..len]);
        if len < buf.len() {
            data.extend_from_slice(&buf[len..]);
        }
        // TODO: random delay
        // TODO: simulate buffer, write will not take effect until flush or close
        Ok(())
    }

    pub async fn set_len(&self, size: u64) -> Result<()> {
        trace!("file({:?}): set_len={}", self.inode.path, size,);
        let mut data = self.inode.data.write().unwrap();
        data.resize(size as usize, 0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Runtime;
    use std::io::ErrorKind;

    #[test]
    fn create_open_read_write() {
        crate::init_logger();

        let runtime = Runtime::new().unwrap();
        let host = runtime.handle("0.0.0.1:1".parse().unwrap());
        let fs = host.fs().clone();
        let f = host.spawn(async move {
            assert_eq!(
                fs.open("file").await.err().unwrap().kind(),
                ErrorKind::NotFound
            );
            let file = fs.create("file").await.unwrap();
            file.write_all_at(b"hello", 0).await.unwrap();

            let mut buf = [0u8; 10];
            let read_len = file.read_at(&mut buf, 2).await.unwrap();
            assert_eq!(read_len, 3);
            assert_eq!(&buf[..3], b"llo");
            drop(file);

            let rofile = fs.open("file").await.unwrap();
            assert_eq!(
                rofile.write_all_at(b"gg", 0).await.err().unwrap().kind(),
                ErrorKind::PermissionDenied
            );
        });
        runtime.block_on(f).unwrap();
    }
}
