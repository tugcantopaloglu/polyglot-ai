//! On-demand file synchronization

use std::path::PathBuf;
use polyglot_common::FileInfo;
use super::{SyncError, SyncManager};

pub const CHUNK_SIZE: usize = 64 * 1024;

pub struct OnDemandSync<'a> {
    manager: &'a SyncManager,
}

impl<'a> OnDemandSync<'a> {
    pub fn new(manager: &'a SyncManager) -> Self {
        Self { manager }
    }

    pub fn get_files_to_sync(
        &self,
        local_files: &[FileInfo],
        remote_files: &[FileInfo],
    ) -> SyncPlan {
        let mut to_upload = Vec::new();
        let mut to_download = Vec::new();
        let to_delete_local = Vec::new();
        let to_delete_remote = Vec::new();

        for remote in remote_files {
            if !remote.is_directory {
                if !local_files.iter().any(|l| l.path == remote.path) {
                    to_download.push(remote.clone());
                }
            }
        }

        for local in local_files {
            if !local.is_directory {
                if !remote_files.iter().any(|r| r.path == local.path) {
                    to_upload.push(local.clone());
                }
            }
        }

        for local in local_files {
            if local.is_directory {
                continue;
            }
            if let Some(remote) = remote_files.iter().find(|r| r.path == local.path) {
                if !remote.is_directory && local.hash != remote.hash {
                    if local.modified_at > remote.modified_at {
                        to_upload.push(local.clone());
                    } else {
                        to_download.push(remote.clone());
                    }
                }
            }
        }

        SyncPlan {
            to_upload,
            to_download,
            to_delete_local,
            to_delete_remote,
        }
    }

    pub fn read_file_chunks(&self, path: &PathBuf) -> Result<FileChunks, SyncError> {
        let content = self.manager.read_file(path)?;
        Ok(FileChunks::new(content))
    }

    pub fn write_file_chunks(
        &self,
        path: &PathBuf,
        chunks: &mut FileChunkWriter,
    ) -> Result<(), SyncError> {
        if chunks.is_complete() {
            self.manager.write_file(path, chunks.data())?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SyncPlan {
    pub to_upload: Vec<FileInfo>,
    pub to_download: Vec<FileInfo>,
    pub to_delete_local: Vec<FileInfo>,
    pub to_delete_remote: Vec<FileInfo>,
}

impl SyncPlan {
    pub fn is_empty(&self) -> bool {
        self.to_upload.is_empty()
            && self.to_download.is_empty()
            && self.to_delete_local.is_empty()
            && self.to_delete_remote.is_empty()
    }

    pub fn total_operations(&self) -> usize {
        self.to_upload.len()
            + self.to_download.len()
            + self.to_delete_local.len()
            + self.to_delete_remote.len()
    }

    pub fn total_bytes(&self) -> u64 {
        let upload_bytes: u64 = self.to_upload.iter().map(|f| f.size).sum();
        let download_bytes: u64 = self.to_download.iter().map(|f| f.size).sum();
        upload_bytes + download_bytes
    }
}

pub struct FileChunks {
    data: Vec<u8>,
    position: usize,
}

impl FileChunks {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, position: 0 }
    }

    pub fn total_size(&self) -> u64 {
        self.data.len() as u64
    }
}

impl Iterator for FileChunks {
    type Item = (u64, Vec<u8>, bool);

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.data.len() {
            return None;
        }

        let offset = self.position as u64;
        let end = std::cmp::min(self.position + CHUNK_SIZE, self.data.len());
        let chunk = self.data[self.position..end].to_vec();
        let is_last = end >= self.data.len();

        self.position = end;

        Some((offset, chunk, is_last))
    }
}

pub struct FileChunkWriter {
    data: Vec<u8>,
    total_size: u64,
    received: u64,
}

impl FileChunkWriter {
    pub fn new(total_size: u64) -> Self {
        Self {
            data: Vec::with_capacity(total_size as usize),
            total_size,
            received: 0,
        }
    }

    pub fn write_chunk(&mut self, offset: u64, chunk: &[u8]) -> Result<(), SyncError> {
        let end = offset as usize + chunk.len();
        if self.data.len() < end {
            self.data.resize(end, 0);
        }

        self.data[offset as usize..end].copy_from_slice(chunk);
        self.received += chunk.len() as u64;

        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.received >= self.total_size
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn progress(&self) -> f64 {
        if self.total_size == 0 {
            1.0
        } else {
            self.received as f64 / self.total_size as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_chunks() {
        let data = vec![0u8; CHUNK_SIZE * 2 + 100];
        let chunks = FileChunks::new(data.clone());

        let collected: Vec<_> = chunks.collect();
        assert_eq!(collected.len(), 3);
        assert!(!collected[0].2);
        assert!(!collected[1].2);
        assert!(collected[2].2);
    }

    #[test]
    fn test_chunk_writer() {
        let mut writer = FileChunkWriter::new(1000);
        writer.write_chunk(0, &vec![1u8; 500]).unwrap();
        assert!(!writer.is_complete());
        writer.write_chunk(500, &vec![2u8; 500]).unwrap();
        assert!(writer.is_complete());
        assert_eq!(writer.data().len(), 1000);
    }
}
