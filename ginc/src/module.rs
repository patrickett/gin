use crate::GIN_FILE_EXT;
use crate::frontend::GinLexer;
use crate::frontend::parser::token_parser;
use crate::frontend::prelude::{ParsedFile, ParsedFolder};
use ariadne::{Label, Report, ReportKind};
use chumsky::Parser;
use chumsky::input::{Input, Stream};
use chumsky::span::Span;
use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

pub trait ToGin {
    fn to_gin_file(self) -> Option<GinFile>;
    fn to_gin_folder(self) -> Option<GinFolder>;
    fn to_gin(self) -> Option<Either<GinFile, GinFolder>>;
}

#[derive(Debug)]
pub struct GinFile(PathBuf);
#[derive(Debug)]
pub struct GinFolder(PathBuf);

pub trait Parsable {
    type Value;
    fn parse(&self) -> Self::Value;
}

impl Parsable for Either<GinFile, GinFolder> {
    type Value = Result<Either<ParsedFile, ParsedFolder>, ()>;

    fn parse(&self) -> Self::Value {
        match self {
            Either::Left(file) => Ok(Either::Left(file.parse().expect("failed to parse a file"))),
            Either::Right(folder) => Ok(Either::Right(folder.parse().expect("msg"))),
        }
    }
}

impl GinFolder {
    #[inline]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Parsable for GinFile {
    type Value = Result<ParsedFile, ()>;

    fn parse(&self) -> Self::Value {
        let source_code = fs::read_to_string(self.0.clone()).expect("file had content");
        // let filename = self
        //     .file_name()
        //     .expect("has filename")
        //     .to_str()
        //     .expect("msg")
        //     .to_string();

        let token_iter = GinLexer::new(&source_code).map(|(tok, span)| (tok, span.into())); // Range -> SimpleSpan

        let token_stream = Stream::from_iter(token_iter)
            .map((0..source_code.len()).into(), |(t, s): (_, _)| (t, s));

        let parser = token_parser();

        let (maybe_output, errors) = parser.parse(token_stream).into_output_errors();

        // can only have ast when no errors
        debug_assert!(
            (maybe_output.is_none() && !errors.is_empty())
                || (maybe_output.is_some() && errors.is_empty())
        );

        if let Some(parsed_file) = maybe_output {
            Ok(parsed_file)
        } else {
            let filename = self.0.file_name().unwrap().to_str().unwrap().to_string();

            let mut cache = ariadne::sources([(filename.clone(), &source_code)]);

            for err in errors.into_iter() {
                let span = err.span();
                let (start, end) = (span.start(), span.end());

                let ariadne_span = (filename.clone(), Range { start, end });
                let msg = format!("{err:?}");

                let report = Report::build(
                    ReportKind::Custom("error", ariadne::Color::Red),
                    ariadne_span.clone(),
                )
                .with_message(msg.clone())
                // TODO: better error msgs
                .with_label(Label::new(ariadne_span).with_message("here"))
                .finish();

                report.eprint(&mut cache).unwrap();
            }
            // Err(errors)
            Err(())
        }
    }
}

impl Parsable for GinFolder {
    type Value = Result<ParsedFolder, ()>;

    fn parse(&self) -> Self::Value {
        let (files, subfolders) = self
            .0
            .read_dir()
            .expect("read dir")
            .filter_map(|a| {
                a.ok()
                    .map(|d| (d.path(), d.path().to_gin().map(|g| g.parse())))
            })
            .fold(
                (BTreeMap::new(), BTreeMap::new()),
                |(mut files, mut folders), (path, maybe_gin)| {
                    if let Some(res) = maybe_gin {
                        match res {
                            Ok(gin) => match gin {
                                Either::Left(file) => {
                                    files.insert(
                                        PathBuf::from(path.file_name().expect("msg")),
                                        file,
                                    );
                                }
                                Either::Right(folder) => {
                                    folders.insert(path, folder);
                                }
                            },
                            Err(_) => todo!(),
                        }
                    }
                    (files, folders)
                },
            );

        Ok(ParsedFolder { files, subfolders })
    }
}

impl ToGin for PathBuf {
    fn to_gin_folder(self) -> Option<GinFolder> {
        if self.is_dir() {
            Some(GinFolder(self))
        } else {
            None
        }
    }

    fn to_gin_file(self) -> Option<GinFile> {
        if self.is_file()
            && let Some(ext) = self.extension()
            && ext == GIN_FILE_EXT
        {
            Some(GinFile(self))
        } else {
            None
        }
    }

    fn to_gin(self) -> Option<Either<GinFile, GinFolder>> {
        self.clone()
            .to_gin_folder()
            .map(Either::<GinFile, GinFolder>::Right)
            .or_else(|| self.to_gin_file().map(Either::<GinFile, GinFolder>::Left))
    }
}
