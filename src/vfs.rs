use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Formatter};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::BufMut;
use log::warn;
use serde::Deserialize;
use thiserror::Error;

pub const DOMAINS_SUBDIR: &str = "domains";
pub const RESOURCES_SUBDIR: &str = "files";
pub const TMP_SUBDIR: &str = ".tmp";
pub const VERSIONS_SUBDIR: &str = "versions";
pub const DRAFTS_SUBDIR: &str = "drafts";
pub const ECMA_SUBDIR: &str = "ecma";
pub const PLUGINS_SUBDIR: &str = "plugins";

pub type Result<T> = std::result::Result<T, VfsErr>;

#[derive(Debug, Error)]
pub enum VfsErr {
    #[error("Domain not found - {0}")]
    Domain(String),
    #[error("File not found - {0}")]
    FileNotFound(String),
    #[error("Schema file not found - {0}")]
    SchemaFileNotFound(String),
    #[error("Absolute file paths not supported - {0}")]
    AbsolutePathNotSupported(String),
    #[error("Dot paths not supported - {0}")]
    DotPathsNotSupported(String),
    #[error("Error parsing JSON - {0}")]
    JsonErr(serde_json::Error),
    #[error("IO error - {0}")]
    Io(std::io::Error),
    #[error("IO error - {0}")]
    StripPrefixErr(std::path::StripPrefixError),
    #[error("IO error - {0}")]
    Utf8(std::string::FromUtf8Error),
}

#[derive(Debug, Deserialize)]
pub struct DomainOptions {
    pub service_id: i64,
    pub version: String,
    pub is_draft: bool,
}

///[Vfs] i.e. virtual file system is specifically designed to constrain access to the file system via API requests
/// whilst also making the access mechanism abstract away from the low level OS FS APIs.
/// Specifically [Vfs] is written to provide access to a structure which assumes multiple APIs are served from a single root directory.
///
/// [BoundVfs] is then used to ensure that when one of these are accessed, all API calls are bound to the specific service ID bounded by it.
/// The underlying filesystem follows this basic format:
/// ```yaml
/// services:
///   domains:
///     my-api.apps.hypi.app - file name is the domain, line 1 is the service ID, line 2 is the version
///   service1:
///     files:
///       file1.png - the static files are served from here but permission checks are done before serving
///       file2.txt
///     versions:
///       v1
///       v2
///       v3
///         schema.xml - required
///         endpoint1.xml
///         endpoint2.xml
///         table1.xml
///         table2.xml
/// ```
pub trait Vfs: Sync + Send {
    ///A base directory against which all paths are [resolve]d.
    fn root(&self) -> &PathBuf;
    fn resolve(&self, child: &str) -> Result<PathBuf> {
        let root = self.root();
        let child_path = Path::new(child);
        //VERY important - root.join below is not safe if child is absolute
        //because join replaces root with child if child is absolute
        if child_path.is_absolute() {
            Err(VfsErr::AbsolutePathNotSupported(child.to_owned()))
        } else if child.contains("./") || child.contains("..") {
            Err(VfsErr::DotPathsNotSupported(child.to_owned()))
        } else {
            let resolved = root.join(child_path);
            let res_str = resolved.to_string_lossy().to_string();
            //note we don't call resolved.canonicalize() because we don't want to hit the file system
            //this resolve is used in all implementations of the Vfs which is not necessarily resolved from disk
            if res_str.starts_with(&root.to_string_lossy().to_string())
            /* root.to_string_lossy().to_string().contains(&res_str)*/
            {
                Ok(resolved)
            } else {
                //somehow the resolved path broke out from under root, don't allow it to continue
                Err(VfsErr::DotPathsNotSupported(child.to_owned()))
            }
        }
    }
    fn domain_file(&self, domain: &str) -> Result<PathBuf> {
        self.resolve(format!("{}/{}", DOMAINS_SUBDIR, domain).as_str())
    }
    fn resource_dir(&self, service_id: i64) -> Result<PathBuf> {
        let dir = self.resolve(format!("{}/{}", service_id, RESOURCES_SUBDIR).as_str())?;
        fs::create_dir_all(dir.clone()).map_err(VfsErr::Io)?;
        Ok(dir)
    }
    fn plugins_dir(&self, service_id: i64) -> Result<PathBuf> {
        let dir = self.resolve(format!("{}/{}", service_id, PLUGINS_SUBDIR).as_str())?;
        fs::create_dir_all(dir.clone()).map_err(VfsErr::Io)?;
        Ok(dir)
    }
    fn tmp_dir(&self, service_id: i64) -> Result<PathBuf> {
        let dir = self.resolve(format!("{}/{}", service_id, TMP_SUBDIR).as_str())?;
        fs::create_dir_all(dir.clone()).map_err(VfsErr::Io)?;
        Ok(dir)
    }
    fn resource_file(&self, service_id: i64, name: &str) -> Result<PathBuf> {
        let mut path = self.resource_dir(service_id)?;
        path.push(name);
        Ok(path)
    }
    fn schema_file(&self, service_id: i64, is_draft: bool, version: &str, file: &str) -> Result<PathBuf> {
        self.resolve(format!("{}/{}/{}/{}", service_id, if is_draft { DRAFTS_SUBDIR } else { VERSIONS_SUBDIR }, version, file).as_str())
    }
    fn ecma_dir(&self, service_id: i64, is_draft: bool, version: &str) -> Result<PathBuf> {
        self.resolve(
            format!(
                "{}/{}/{}/{}",
                service_id, if is_draft { DRAFTS_SUBDIR } else { VERSIONS_SUBDIR }, version, ECMA_SUBDIR
            )
                .as_str(),
        )
    }
    fn read(&self, file: PathBuf) -> Result<Box<dyn Read + '_>>;
    fn open_with(&self, file: PathBuf, opts: OpenOptions) -> Result<Box<dyn VfsFile>>;
    fn read_domain_file(&self, domain: &str) -> Result<DomainOptions> {
        match self.domain_file(domain) {
            Ok(file) => {
                let mut data = vec![];
                let mut input = self.read(file)?;
                let mut buffer = [0; 1024];
                while let Ok(n) = input.read(&mut buffer).map_err(VfsErr::Io) {
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[0..n]);
                }
                Ok(serde_json::from_slice(&data).map_err(VfsErr::JsonErr)?)
            }
            Err(e) => Err(e),
        }
    }
    fn read_resource_file(&self, service_id: i64, filename: &str) -> Result<Box<dyn Read + '_>> {
        match self.resource_file(service_id, filename) {
            Ok(file) => self.read(file),
            Err(e) => Err(e),
        }
    }
    fn read_schema_file(&self, service_id: i64, is_draft: bool, version: &str, filename: &str) -> Result<String> {
        match self.schema_file(service_id, is_draft, version, filename) {
            Ok(file) => {
                let mut data = vec![];
                let mut input = self.read(file)?;
                let mut buffer = [0; 1024];
                while let Ok(n) = input.read(&mut buffer).map_err(VfsErr::Io) {
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[0..n]);
                }
                Ok(String::from_utf8(data).map_err(VfsErr::Utf8)?)
            }
            Err(e) => Err(e),
        }
    }
    fn read_ecma<'a>(&'a self, service_id: i64, is_draft: bool, version: &str) -> Result<DirStream<'a, Self>> {
        let dir = self.ecma_dir(service_id, is_draft, version)?;
        self.dir_stream(dir)
    }
    fn dir_stream<'a>(&'a self, dir: PathBuf) -> Result<DirStream<'a, Self>> {
        if dir.to_string_lossy().contains("..") {
            warn!("ECMA script path cannot contain '..' i.e. must be absolute, full path");
            return Err(VfsErr::DotPathsNotSupported(format!(
                "ECMA script path can't have .. in {}",
                dir.to_string_lossy()
            )));
        }
        match self.read_dir(&dir) {
            Ok(read_dir) => {
                let mut stream: DirStream<'a, Self> = DirStream {
                    base: dir,
                    buf: VecDeque::new(),
                    vfs: self,
                };
                stream.buf.push_back(read_dir);
                Ok(stream)
            }
            Err(e) => Err(e),
        }
    }
    fn read_dir(&self, dir: &PathBuf) -> Result<VirtualReadDir>;
}

pub struct VirtualReadDir {
    inner: Box<dyn Iterator<Item=PathBuf>>,
}

impl Iterator for VirtualReadDir {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

pub struct DirStream<'a, F>
    where
        F: Vfs + ?Sized,
{
    base: PathBuf,
    buf: VecDeque<VirtualReadDir>,
    vfs: &'a F,
}

impl<'a, F: Vfs> Iterator for DirStream<'a, F> {
    type Item = Result<(PathBuf, PathBuf)>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(dir) = self.buf.back_mut() {
            if let Some(path) = dir.next() {
                //can't use canonicalize because it goes to the filesystem
                // let path = match path.canonicalize().map_err(VfsErr::Io) {
                //     Ok(p) => p,
                //     Err(e) => return Some(Err(e)),
                // };
                if path.to_string_lossy().contains("..") {
                    warn!(
                        "Skipping path {} because it contains '..'",
                        path.to_string_lossy()
                    );
                    return self.next();
                }
                if path.is_dir() {
                    match self.vfs.read_dir(&path) {
                        Ok(child) => {
                            self.buf.push_front(child);
                            self.next()
                        }
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    if path.starts_with(&self.base) {
                        let filename = match path
                            .strip_prefix(&self.base)
                            .map_err(VfsErr::StripPrefixErr)
                        {
                            Ok(p) => p,
                            Err(e) => return Some(Err(e)),
                        };
                        Some(Ok((filename.to_owned(), path)))
                    } else {
                        //Some(Err(VfsErr::Generic(format!())))
                        self.next() //silently skip files that are not in the service's base directory
                    }
                }
            } else {
                self.buf.pop_back();
                self.next()
            }
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct FilesystemVfs {
    ///The absolute path to the directory where the services are kept
    ///This is important because we ensure that all operations are a sub-directory of this
    services_dir: PathBuf,
}

pub trait VfsFile: Read + Write + Seek {
    fn path(&self) -> PathBuf;
    fn clone(&self) -> Result<Box<dyn VfsFile>>;
}

impl dyn VfsFile {
    pub fn save_to<F>(&self, fs: Arc<BoundVfs<F>>, new_name: Option<String>) -> Result<String>
        where
            F: Vfs,
    {
        fs.save_to(self, new_name)
    }
    pub fn discard<F>(&self, fs: Arc<BoundVfs<F>>) -> Result<()>
        where
            F: Vfs,
    {
        fs.discard(self)
    }
}

impl Debug for dyn VfsFile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("VfsFile")
    }
}

pub struct VfsFileSystemFile(File, PathBuf);

impl VfsFile for VfsFileSystemFile {
    fn path(&self) -> PathBuf {
        self.1.clone()
    }
    fn clone(&self) -> Result<Box<dyn VfsFile>> {
        let mut opts = OpenOptions::new();
        opts.read(true);
        Ok(Box::new(VfsFileSystemFile(
            opts.open(self.1.clone()).map_err(VfsErr::Io)?,
            self.1.clone(),
        )))
    }
}

impl Read for VfsFileSystemFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for VfsFileSystemFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Seek for VfsFileSystemFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl Vfs for FilesystemVfs {
    fn root(&self) -> &PathBuf {
        &self.services_dir
    }

    fn read(&self, file: PathBuf) -> Result<Box<dyn Read + '_>> {
        if file.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot read file with .. in path {}",
                file.to_string_lossy()
            )));
        }
        Ok(Box::new(File::open(file).map_err(VfsErr::Io)?))
    }
    fn open_with(&self, path: PathBuf, opts: OpenOptions) -> Result<Box<dyn VfsFile>> {
        if path.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot open file with .. in path {}",
                path.to_string_lossy()
            )));
        }
        let file = opts.open(path.clone()).map_err(VfsErr::Io)?;
        Ok(Box::new(VfsFileSystemFile(file, path)))
    }

    fn read_dir(&self, dir: &PathBuf) -> Result<VirtualReadDir> {
        if dir.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot read dir with .. in path {}",
                dir.to_string_lossy()
            )));
        }
        let it = fs::read_dir(dir).map_err(VfsErr::Io)?;
        let it = it.map(|v| v.map(|e| e.path())).flatten();
        let it: Box<dyn Iterator<Item=PathBuf>> = Box::new(it);
        Ok(VirtualReadDir { inner: it })
    }
}

impl FilesystemVfs {
    pub fn new(services_dir: String) -> Self {
        FilesystemVfs {
            services_dir: PathBuf::from(services_dir),
        }
    }
}

#[allow(unused)]
pub struct MemVfsFile {
    path: PathBuf,
    data: Vec<u8>,
    offset: usize,
}

impl Seek for MemVfsFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::Start(_start) => {}
            SeekFrom::End(_end) => {}
            SeekFrom::Current(_current) => {}
        }
        todo!();
        // Ok(0)
    }
}

impl Read for MemVfsFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = self.offset;
        let mut end = start + buf.len();
        let buf_len = self.data.len();
        if end >= buf_len {
            end = buf_len;
        }
        if start >= end {
            return Ok(0);
        }
        let slice = &self.data[start..end];
        let read = end - start;
        buf[0..read].clone_from_slice(slice);
        self.offset = end;
        Ok(read)
    }
}

impl Write for MemVfsFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.data.put_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        println!(
            "MemVfsFile::flush:{}",
            String::from_utf8(self.data.clone()).unwrap()
        );
        Ok(())
    }
}

impl VfsFile for MemVfsFile {
    fn path(&self) -> PathBuf {
        self.path.clone()
    }
    fn clone(&self) -> Result<Box<dyn VfsFile>> {
        Ok(Box::new(MemVfsFile {
            path: self.path.clone(),
            data: self.data.clone(),
            offset: 0,
        }))
    }
}

#[derive(Clone)]
pub struct MemoryVfs {
    pub root: PathBuf,
    pub data: HashMap<String, String>,
}

impl Vfs for MemoryVfs {
    fn root(&self) -> &PathBuf {
        &self.root
    }

    fn read(&self, file: PathBuf) -> Result<Box<dyn Read + '_>> {
        if file.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot read file with .. in path {}",
                file.to_string_lossy()
            )));
        }
        match self.data.get(file.to_string_lossy().as_ref()) {
            Some(data) => {
                let data: &[u8] = data.as_bytes();
                Ok(Box::new(data))
            }
            None => Err(VfsErr::FileNotFound(format!(
                "File not found - {}",
                file.to_string_lossy()
            ))),
        }
    }

    fn open_with(&self, file: PathBuf, _opts: OpenOptions) -> Result<Box<dyn VfsFile>> {
        if file.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot read file with .. in path {}",
                file.to_string_lossy()
            )));
        }
        match self.data.get(file.to_string_lossy().as_ref()) {
            Some(data) => {
                let data: &[u8] = data.as_bytes();
                Ok(Box::new(MemVfsFile {
                    path: file,
                    data: Vec::from(data),
                    offset: 0,
                }))
            }
            None => {
                //we assume write/append and create it - means there's a different behaviour with in-memory vs disk
                Ok(Box::new(MemVfsFile {
                    path: file,
                    data: vec![],
                    offset: 0,
                }))
                // Err(VfsErr::FileNotFound(format!(
                //     "File not found - {}",
                //     file.to_string_lossy()
                // )))
            }
        }
    }

    fn read_dir(&self, dir: &PathBuf) -> Result<VirtualReadDir> {
        if dir.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot read dir with .. in path {}",
                dir.to_string_lossy()
            )));
        }
        let it: Vec<_> = self
            .data
            .keys()
            .map(PathBuf::from)
            .skip_while(|path| !path.starts_with(dir))
            .collect();
        Ok(VirtualReadDir {
            inner: Box::new(it.into_iter()),
        })
    }
}

pub struct BoundVfs<F>
    where
        F: Vfs,
{
    pub options: DomainOptions,
    pub vfs: Arc<F>,
}

impl<F> BoundVfs<F>
    where
        F: Vfs,
{
    pub fn new(options: DomainOptions, vfs: Arc<F>) -> BoundVfs<F> {
        Self { options, vfs }
    }
    pub fn read_schema_file(&self, name: &str) -> Result<String> {
        self.vfs
            .read_schema_file(self.options.service_id, self.options.is_draft, self.options.version.as_str(), name)
    }

    pub fn ecma_files(&self) -> Result<DirStream<F>> {
        self.vfs
            .read_ecma(self.options.service_id, self.options.is_draft, self.options.version.as_str())
    }

    pub fn read_ecma_file(&self, mut file: PathBuf) -> Result<String> {
        if file.starts_with("./") {
            file = file
                .strip_prefix("./")
                .map_err(VfsErr::StripPrefixErr)?
                .to_owned();
        }
        let mut path = self
            .vfs
            .ecma_dir(self.options.service_id, self.options.is_draft, self.options.version.as_str())?;
        path.push(file);
        let mut read = self.vfs.read(path)?;
        let mut str = String::new();
        read.read_to_string(&mut str).map_err(VfsErr::Io)?;
        Ok(str)
    }

    pub fn resource_dir(&self) -> Result<PathBuf> {
        self.vfs.resource_dir(self.options.service_id)
    }

    pub fn resolve_resource(&self, mut file: PathBuf) -> Result<PathBuf> {
        if file.starts_with("./") {
            file = file
                .strip_prefix("./")
                .map_err(VfsErr::StripPrefixErr)?
                .to_owned();
        } else if file.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot open file with .. in path {}",
                file.to_string_lossy()
            )));
        }
        let mut path = self.vfs.resource_dir(self.options.service_id)?;
        path.push(file);
        Ok(path)
    }
    pub fn resolve_plugin(&self, mut file: PathBuf) -> Result<PathBuf> {
        if file.starts_with("./") {
            file = file
                .strip_prefix("./")
                .map_err(VfsErr::StripPrefixErr)?
                .to_owned();
        } else if file.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot open file with .. in path {}",
                file.to_string_lossy()
            )));
        }
        let mut path = self.vfs.plugins_dir(self.options.service_id)?;
        path.push(file);
        Ok(path)
    }
    pub fn open(&self, mut file: PathBuf, opts: OpenOptions) -> Result<Box<dyn VfsFile>> {
        if file.starts_with("./") {
            file = file
                .strip_prefix("./")
                .map_err(VfsErr::StripPrefixErr)?
                .to_owned();
        } else if file.to_string_lossy().contains("..") {
            return Err(VfsErr::DotPathsNotSupported(format!(
                "Cannot open file with .. in path {}",
                file.to_string_lossy()
            )));
        }
        self.vfs.open_with(self.resolve_resource(file)?, opts)
    }

    pub fn discard<I>(&self, _file: &I) -> Result<()>
        where
            I: VfsFile + ?Sized,
    {
        todo!();
        // Ok(())
    }
    pub fn save_to<I>(&self, file: &I, new_name: Option<String>) -> Result<String>
        where
            I: VfsFile + ?Sized,
    {
        let mut other_path = file.path();
        let mut path = self.vfs.resource_dir(self.options.service_id)?;
        if other_path.starts_with(&path) {
            other_path = PathBuf::from(
                other_path
                    .strip_prefix(&path)
                    .map_err(VfsErr::StripPrefixErr)?,
            );
        }
        if other_path.starts_with(TMP_SUBDIR) {
            other_path = PathBuf::from(
                other_path
                    .strip_prefix(TMP_SUBDIR)
                    .map_err(VfsErr::StripPrefixErr)?,
            )
        }
        path.push(other_path);
        if let Some(file_name) = new_name {
            path.set_file_name(file_name);
        }
        let name = if let Some(name) = path.file_name().map(|v| v.to_str()).flatten() {
            name.to_string()
        } else {
            file.path()
                .to_string_lossy()
                .split("/")
                .last()
                .unwrap()
                .to_string()
        };
        fs::rename(file.path(), path).map_err(VfsErr::Io)?;
        Ok(name)
    }
}
