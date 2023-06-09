use flate2::{write::GzEncoder, Compression};
use humansize::{make_format, DECIMAL};
use std::{
    collections::hash_map::DefaultHasher,
    fmt,
    fs::File,
    hash::Hasher,
    io::{copy, BufReader, Seek, SeekFrom},
};
use tar::Archive;
use tempfile::NamedTempFile;

/// The compression level to use when compressing files (0-9)
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum CompressionLevel {
    /// No compression (0)
    None,
    /// Fast compression (1)
    Fast,
    /// Default compression (6)
    Default,
    /// Maximum compression (9)
    Maximum,
    /// Custom compression level (0-9)
    Custom(u32),
}

impl AsRef<CompressionLevel> for CompressionLevel {
    fn as_ref(&self) -> &CompressionLevel {
        self
    }
}

impl From<&CompressionLevel> for u32 {
    fn from(value: &CompressionLevel) -> Self {
        use CompressionLevel::{Custom, Default, Fast, Maximum, None};
        match value {
            None => 0,
            Fast => 1,
            Default => 6,
            Maximum => 9,
            Custom(level) => *level,
        }
    }
}

impl From<CompressionLevel> for u32 {
    fn from(value: CompressionLevel) -> Self {
        use CompressionLevel::{Custom, Default, Fast, Maximum, None};
        match value {
            None => 0,
            Fast => 1,
            Default => 6,
            Maximum => 9,
            Custom(level) => level,
        }
    }
}

impl TryFrom<CompressionLevel> for Compression {
    type Error = String;

    fn try_from(value: CompressionLevel) -> Result<Self, Self::Error> {
        use CompressionLevel::{Custom, Default, Fast, Maximum, None};
        match value {
            None => Ok(Compression::none()),
            Fast => Ok(Compression::fast()),
            Default => Ok(Compression::default()),
            Maximum => Ok(Compression::best()),
            Custom(level) => {
                if level > 9 {
                    Err(format!(
                        "Invalid compression level: {level}, must be between 0 and 9"
                    ))
                } else {
                    Ok(Compression::new(level))
                }
            }
        }
    }
}

impl TryFrom<&CompressionLevel> for Compression {
    type Error = String;

    fn try_from(value: &CompressionLevel) -> Result<Self, Self::Error> {
        use CompressionLevel::{Custom, Default, Fast, Maximum, None};
        match value {
            None => Ok(Compression::none()),
            Fast => Ok(Compression::fast()),
            Default => Ok(Compression::default()),
            Maximum => Ok(Compression::best()),
            Custom(level) => {
                if *level > 9 {
                    Err(format!(
                        "Invalid compression level: {level}, must be between 0 and 9"
                    ))
                } else {
                    Ok(Compression::new(*level))
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct ArchiveInfo {
    input_size: u64,
    output_size: u64,
    ratio: f64,
}

impl ArchiveInfo {
    /// Get the input size without formatting
    #[must_use]
    pub fn input_size(&self) -> u64 {
        self.input_size
    }

    /// Get the input size in a human readable format
    ///
    /// # Example
    ///
    /// ```
    /// use comprexor::ArchiveInfo;
    ///
    /// let archive_data = ArchiveInfo {
    ///    input_size: 1000,
    ///    output_size: 1000,
    ///    ratio: 1.0,
    /// };
    ///
    /// assert_eq!(archive_data.input_size_formatted(), "1.0 kB");
    /// ```
    #[must_use]
    pub fn input_size_formatted(&self) -> String {
        let formatter = make_format(DECIMAL);
        formatter(self.input_size)
    }

    /// Get the output size without formatting
    #[must_use]
    pub fn output_size(&self) -> u64 {
        self.output_size
    }

    /// Get the output size in a human readable format
    ///
    /// # Example
    ///
    /// ```
    /// use comprexor::ArchiveInfo;
    ///
    /// let archive_data = ArchiveInfo {
    ///   input_size: 1000,
    ///   output_size: 1000,
    ///   ratio: 1.0,
    /// };
    ///
    /// assert_eq!(archive_data.output_size_formatted(), "1.0 kB");
    /// ```
    #[must_use]
    pub fn output_size_formatted(&self) -> String {
        let formatter = make_format(DECIMAL);
        formatter(self.output_size)
    }

    /// Get the ratio without formatting
    #[must_use]
    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// Get the ratio formatted to the given number of decimals
    ///
    /// # Example
    ///
    /// ```
    /// use comprexor::ArchiveInfo;
    ///
    /// let archive_data = ArchiveInfo {
    ///     input_size: 1000,
    ///     output_size: 1000,
    ///     ratio: 1.0,
    /// };
    ///
    /// assert_eq!(archive_data.ratio_formatted(5), "1.00000");
    /// assert_eq!(archive_data.ratio_formatted(2), "1.00");
    /// ```
    #[must_use]
    pub fn ratio_formatted(&self, num_decimals: u8) -> String {
        format!(
            "{:.decimals$}",
            self.ratio,
            decimals = num_decimals as usize
        )
    }
}

trait ArchiveExt {
    fn gen_hashed_name<T>(input: &T) -> String
    where
        T: std::hash::Hash + fmt::Display,
    {
        let mut hasher = DefaultHasher::new();
        input.hash(&mut hasher);
        format!("{}-{}", hasher.finish(), "tar-gen.tar")
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Hash)]
pub struct Compressor<'a> {
    input: &'a str,
    output: &'a str,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Hash)]
pub struct Extractor<'a> {
    input: &'a str,
    output: &'a str,
}

impl<'a> ArchiveExt for Compressor<'a> {}
impl<'a> ArchiveExt for Extractor<'a> {}

impl<'a> Extractor<'a> {
    #[must_use]
    /// Create a new extractor with the given input and output
    ///
    /// # Example
    /// ```
    /// use comprexor::Extractor;
    ///
    /// let extractor = Extractor::new("./compacted-archive.tar.gz", "./output-folder-or-file");
    /// extractor.extract().unwrap();
    /// ```
    pub fn new(input: &'a str, output: &'a str) -> Extractor<'a> {
        Self { input, output }
    }

    /// Decompress the input file to the output file
    ///
    /// # Example
    ///
    /// ```
    /// use comprexor::Extractor;
    ///
    /// let extractor = Extractor::new("./compacted-archive.tar.gz", "./output-folder-or-file");
    /// extractor.extract().unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// This function will return an error if the input file is not a valid gzip file or something goes wrong while decompressing
    pub fn extract(&self) -> Result<ArchiveInfo, std::io::Error> {
        let archive_data = self.extract_internal()?;
        Ok(archive_data)
    }

    fn extract_internal(&self) -> Result<ArchiveInfo, std::io::Error> {
        let input_file = BufReader::new(std::fs::File::open(self.input)?);
        let input_size = std::fs::metadata(self.input)?.len();
        let mut tmpfile = tempfile::tempfile()?;

        let mut decoder = flate2::read::GzDecoder::new(input_file);
        copy(&mut decoder, &mut tmpfile)?;
        tmpfile.seek(SeekFrom::Start(0))?;
        let output_size = tmpfile.metadata()?.len();

        let mut archive = Archive::new(tmpfile);
        archive.unpack(self.output)?;

        Ok(ArchiveInfo {
            input_size,
            output_size,
            ratio: output_size as f64 / input_size as f64,
        })
    }
}

impl<'a> Compressor<'a> {
    #[must_use]
    /// Creates a new compressor with the given input and output
    ///
    /// # Example
    ///
    /// ```
    /// use comprexor::{CompressionLevel, Compressor};
    ///
    /// let compressor = Compressor::new("./folder-or-file-to-compress", "./compacted-archive.tar.gz");
    /// compressor.compress(CompressionLevel::Maximum).unwrap();
    /// ```
    pub fn new(input: &'a str, output: &'a str) -> Compressor<'a> {
        Self { input, output }
    }

    /// Compress the input file or folder to the output location
    ///
    /// You can choose the compression level with the `CompressionLevel` enum
    ///
    /// The compression level can be:
    ///
    /// - `CompressionLevel::None`
    /// - `CompressionLevel::Fast`
    /// - `CompressionLevel::Default`
    /// - `CompressionLevel::Maximum`
    /// - `CompressionLevel::Custom` (you can specify the compression level between 0 and 9)
    ///
    /// # Example
    ///
    /// ```
    /// use comprexor::{CompressionLevel, Compressor};
    ///
    /// let compressor = Compressor::new("./folder-or-file-to-compress", "./compacted-archive.tar.gz");
    /// compressor.compress(CompressionLevel::Maximum).unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// This function will return an error if the input file is not a valid gzip file or something goes wrong while compressing
    pub fn compress<T>(&self, level: T) -> Result<ArchiveInfo, std::io::Error>
    where
        T: AsRef<CompressionLevel>,
    {
        let archive_data = self.compress_with_tar(level.as_ref())?;

        Ok(archive_data)
    }

    fn compress_with_tar(&self, level: &CompressionLevel) -> Result<ArchiveInfo, std::io::Error> {
        let mut tmpfile = NamedTempFile::new()?;
        let mut tar = tar::Builder::new(tmpfile.reopen()?);

        if std::fs::metadata(self.input)?.is_dir() {
            let folder_name = std::path::Path::new(self.input)
                .file_name()
                .ok_or(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Could not get file name from input",
                ))?
                .to_str()
                .ok_or(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Could not convert file name to str",
                ))?;
            tar.append_dir_all(folder_name, self.input)?;
        } else if std::fs::metadata(self.input)?.is_file()
            || std::fs::metadata(self.input)?.is_symlink()
        {
            let mut file = std::fs::File::open(self.input)?;
            tar.append_file(self.input, &mut file)?;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Input is neither a file, symlink or a directory",
            ));
        }

        tar.finish()?;
        tmpfile.seek(SeekFrom::Start(0))?;

        let archive_data = self.compress_internal(&mut tmpfile.reopen()?, level)?;

        // By closing the `TempPath` explicitly, we can check that it has
        // been deleted successfully. If we don't close it explicitly, the
        // file will still be deleted when `file` goes out of scope, but we
        // won't know whether deleting the file succeeded.
        tmpfile.close()?;

        Ok(archive_data)
    }

    fn compress_internal(
        &self,
        input_file: &mut File,
        level: &CompressionLevel,
    ) -> Result<ArchiveInfo, std::io::Error> {
        let input_size = input_file.metadata()?.len();
        let output_file = std::fs::File::create(self.output)?;

        let mut encoder = GzEncoder::new(
            output_file,
            level
                .try_into()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?,
        );
        copy(input_file, &mut encoder)?;
        encoder.finish()?;
        let output_size = std::fs::metadata(self.output)?.len();

        Ok(ArchiveInfo {
            input_size,
            output_size,
            ratio: input_size as f64 / output_size as f64,
        })
    }
}
