use miniz_oxide::deflate::compress_to_vec;

struct ZipEntry {
    name: String,
    crc32: u32,
    uncompressed_size: u32,
    compressed_data: Vec<u8>,
    compression_method: u16,
    local_header_offset: u32,
}

/// Deterministic, sans-IO ZIP writer with fixed 1980-01-01 timestamp for byte-identical native/WASM output.
#[derive(Default)]
pub struct DeterministicZipWriter {
    buf: Vec<u8>,
    entries: Vec<ZipEntry>,
}

impl DeterministicZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, name: &str, data: &[u8], deflate: bool) {
        let crc = crc32fast::hash(data);
        let uncompressed_size = data.len() as u32;
        let (compression_method, compressed_data) = if deflate && !data.is_empty() {
            (8u16, compress_to_vec(data, 6))
        } else {
            (0u16, data.to_vec())
        };
        let compressed_size = compressed_data.len() as u32;
        let local_header_offset = self.buf.len() as u32;
        let name_bytes = name.as_bytes();

        // Local file header signature (0x04034b50)
        self.buf.extend_from_slice(&0x04034b50u32.to_le_bytes());
        self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed (2.0)
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // general purpose bit flag
        self.buf
            .extend_from_slice(&compression_method.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod time = 00:00:00
        self.buf.extend_from_slice(&0x0021u16.to_le_bytes()); // mod date = 1980-01-01
        self.buf.extend_from_slice(&crc.to_le_bytes());
        self.buf.extend_from_slice(&compressed_size.to_le_bytes());
        self.buf.extend_from_slice(&uncompressed_size.to_le_bytes());
        self.buf
            .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        self.buf.extend_from_slice(name_bytes);
        self.buf.extend_from_slice(&compressed_data);

        self.entries.push(ZipEntry {
            name: name.to_string(),
            crc32: crc,
            uncompressed_size,
            compressed_data,
            compression_method,
            local_header_offset,
        });
    }

    pub fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.buf.len() as u32;
        for entry in &self.entries {
            let name_bytes = entry.name.as_bytes();
            let compressed_size = entry.compressed_data.len() as u32;

            // Central directory file header signature (0x02014b50)
            self.buf.extend_from_slice(&0x02014b50u32.to_le_bytes());
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // version made by
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // flags
            self.buf
                .extend_from_slice(&entry.compression_method.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
            self.buf.extend_from_slice(&0x0021u16.to_le_bytes()); // mod date
            self.buf.extend_from_slice(&entry.crc32.to_le_bytes());
            self.buf.extend_from_slice(&compressed_size.to_le_bytes());
            self.buf
                .extend_from_slice(&entry.uncompressed_size.to_le_bytes());
            self.buf
                .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // comment len
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // disk number start
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            self.buf.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            self.buf
                .extend_from_slice(&entry.local_header_offset.to_le_bytes());
            self.buf.extend_from_slice(name_bytes);
        }

        let cd_size = (self.buf.len() as u32) - cd_offset;
        let count = self.entries.len() as u16;

        // End of central directory record (0x06054b50)
        self.buf.extend_from_slice(&0x06054b50u32.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // disk number
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        self.buf.extend_from_slice(&count.to_le_bytes());
        self.buf.extend_from_slice(&count.to_le_bytes());
        self.buf.extend_from_slice(&cd_size.to_le_bytes());
        self.buf.extend_from_slice(&cd_offset.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // comment length

        self.buf
    }
}
