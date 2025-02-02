//! バイトコードからオブジェクトを復元する
use std::process;
use std::string::FromUtf8Error;

use crate::cache::Cache;
use crate::codeobj::CodeObj;
use crate::config::{ErgConfig, Input};
use crate::error::{ErrorCore, ErrorKind, Location};
use crate::serialize::DataTypePrefix;
use crate::traits::HasType;
use crate::ty::{TyParam, Type};
use crate::value::ValueObj;
use crate::{fn_name, switch_lang};
use crate::{RcArray, Str};

#[derive(Debug)]
pub struct DeserializeError {
    pub errno: usize,
    pub caused_by: Str,
    pub desc: Str,
}

impl From<std::io::Error> for DeserializeError {
    fn from(err: std::io::Error) -> Self {
        Self::new(0, "io::Error::into", err.to_string())
    }
}

impl From<FromUtf8Error> for DeserializeError {
    fn from(err: FromUtf8Error) -> Self {
        Self::new(0, "Str::try_from", err.to_string())
    }
}

impl From<DeserializeError> for ErrorCore {
    fn from(err: DeserializeError) -> Self {
        ErrorCore::new(
            err.errno,
            ErrorKind::ImportError,
            Location::Unknown,
            err.desc,
            Option::<Str>::None,
        )
    }
}

impl DeserializeError {
    pub fn new<S: Into<Str>, T: Into<Str>>(errno: usize, caused_by: S, desc: T) -> Self {
        Self {
            errno,
            caused_by: caused_by.into(),
            desc: desc.into(),
        }
    }

    pub fn file_broken_error() -> Self {
        Self::new(
            0,
            fn_name!(),
            switch_lang!(
                "japanese" => "読み込んだ.pycファイルは破損しています",
                "english" => "the loaded .pyc file is broken",
            ),
        )
    }

    pub fn type_error(expect: &Type, found: &Type) -> Self {
        Self::new(
            0,
            fn_name!(),
            switch_lang!(
                "japanese" => format!(
                    "{}型オブジェクトを予期しましたが、 読み込んだオブジェクトは{}型です",
                    expect, found
                ),
                "english" => format!(
                    "expect a {} object, but the deserialized object is {}",
                    expect, found
                ),
            ),
        )
    }
}

pub type DeserializeResult<T> = Result<T, DeserializeError>;

#[derive(Default)]
pub struct Deserializer {
    str_cache: Cache<str>,
    arr_cache: Cache<[ValueObj]>,
    dict_cache: Cache<[(ValueObj, ValueObj)]>,
}

impl Deserializer {
    pub fn new() -> Self {
        Self {
            str_cache: Cache::new(),
            arr_cache: Cache::new(),
            dict_cache: Cache::new(),
        }
    }

    pub fn run(cfg: ErgConfig) {
        let filename = if let Input::File(f) = cfg.input {
            f
        } else {
            eprintln!("{:?} is not a filename", cfg.input);
            process::exit(1);
        };
        let codeobj = CodeObj::from_pyc(&filename[..])
            .unwrap_or_else(|_| panic!("failed to deserialize {filename}"));
        println!("{}", codeobj.code_info());
    }

    fn get_cached_str(&mut self, s: &str) -> ValueObj {
        ValueObj::Str(self.str_cache.get(s))
    }

    fn get_cached_arr(&mut self, arr: &[ValueObj]) -> ValueObj {
        ValueObj::Array(self.arr_cache.get(arr))
    }

    /// TODO: 使わない？
    pub fn get_cached_dict(&mut self, dict: &[(ValueObj, ValueObj)]) -> ValueObj {
        ValueObj::Dict(self.dict_cache.get(dict))
    }

    pub fn vec_to_bytes<const LEN: usize>(vector: Vec<u8>) -> [u8; LEN] {
        let mut arr = [0u8; LEN];
        for (arr_elem, vec_elem) in arr.iter_mut().zip(vector.iter()) {
            *arr_elem = *vec_elem;
        }
        arr
    }

    pub fn consume<const LEN: usize>(v: &mut Vec<u8>) -> [u8; LEN] {
        Self::vec_to_bytes::<LEN>(v.drain(..LEN).collect::<Vec<_>>())
    }

    pub fn deserialize_u32(v: &mut Vec<u8>) -> u32 {
        u32::from_le_bytes(Self::consume::<4>(v))
    }

    pub fn deserialize_const(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: u32,
    ) -> DeserializeResult<ValueObj> {
        match DataTypePrefix::from(v.remove(0)) {
            DataTypePrefix::Int32 => {
                let bytes = Self::consume::<4>(v);
                Ok(ValueObj::Int(i32::from_le_bytes(bytes)))
            }
            DataTypePrefix::BinFloat => {
                let bytes = Self::consume::<8>(v);
                Ok(ValueObj::Float(f64::from_le_bytes(bytes)))
            }
            DataTypePrefix::ShortAscii | DataTypePrefix::ShortAsciiInterned => {
                let len = v.remove(0);
                let bytes = v.drain(..len as usize).collect();
                Ok(self.get_cached_str(&String::from_utf8(bytes)?))
            }
            DataTypePrefix::Str | DataTypePrefix::Unicode => {
                let len = Self::deserialize_u32(v);
                let bytes = v.drain(..len as usize).collect();
                Ok(self.get_cached_str(&String::from_utf8(bytes)?))
            }
            DataTypePrefix::True => Ok(ValueObj::True),
            DataTypePrefix::False => Ok(ValueObj::False),
            DataTypePrefix::SmallTuple => {
                let len = v.remove(0);
                let mut arr = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    arr.push(self.deserialize_const(v, python_ver)?);
                }
                Ok(self.get_cached_arr(&arr))
            }
            DataTypePrefix::Tuple => {
                let len = Self::deserialize_u32(v);
                let mut arr = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    arr.push(self.deserialize_const(v, python_ver)?);
                }
                Ok(self.get_cached_arr(&arr))
            }
            DataTypePrefix::Code => {
                let argcount = Self::deserialize_u32(v);
                let posonlyargcount = if python_ver >= 3413 {
                    Self::deserialize_u32(v)
                } else {
                    0
                };
                let kwonlyargcount = Self::deserialize_u32(v);
                let nlocals = Self::deserialize_u32(v);
                let stacksize = Self::deserialize_u32(v);
                let flags = Self::deserialize_u32(v);
                let code = self.deserialize_bytes(v)?;
                let consts = self.deserialize_const_vec(v, python_ver)?;
                let names = self.deserialize_str_vec(v, python_ver)?;
                let varnames = self.deserialize_str_vec(v, python_ver)?;
                let freevars = self.deserialize_str_vec(v, python_ver)?;
                let cellvars = self.deserialize_str_vec(v, python_ver)?;
                let filename = self.deserialize_str(v, python_ver)?;
                let name = self.deserialize_str(v, python_ver)?;
                let firstlineno = Self::deserialize_u32(v);
                let lnotab = self.deserialize_bytes(v)?;
                Ok(ValueObj::from(CodeObj::new(
                    argcount,
                    posonlyargcount,
                    kwonlyargcount,
                    nlocals,
                    stacksize,
                    flags,
                    code,
                    consts,
                    names,
                    varnames,
                    freevars,
                    cellvars,
                    filename,
                    name,
                    firstlineno,
                    lnotab,
                )))
            }
            DataTypePrefix::None => Ok(ValueObj::None),
            other => Err(DeserializeError::new(
                0,
                fn_name!(),
                switch_lang!(
                    "japanese" => format!("このオブジェクトは復元できません: {}", other),
                    "english" => format!("cannot deserialize this object: {}", other),
                ),
            )),
        }
    }

    pub fn deserialize_const_vec(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: u32,
    ) -> DeserializeResult<Vec<ValueObj>> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Array(arr) => Ok(arr.to_vec()),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn deserialize_const_array(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: u32,
    ) -> DeserializeResult<RcArray<ValueObj>> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Array(arr) => Ok(arr),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn array_into_const(&mut self, arr: &[ValueObj]) -> ValueObj {
        self.get_cached_arr(arr)
    }

    pub fn try_into_str(&mut self, c: ValueObj) -> DeserializeResult<Str> {
        match c {
            ValueObj::Str(s) => Ok(s),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn deserialize_str_vec(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: u32,
    ) -> DeserializeResult<Vec<Str>> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Array(arr) => {
                let mut strs = Vec::with_capacity(arr.len());
                for c in arr.iter().cloned() {
                    strs.push(self.try_into_str(c)?);
                }
                Ok(strs)
            }
            other => Err(DeserializeError::type_error(
                &Type::array(Type::Str, TyParam::erased(Type::Nat)),
                other.ref_t(),
            )),
        }
    }

    pub fn deserialize_str(&mut self, v: &mut Vec<u8>, python_ver: u32) -> DeserializeResult<Str> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Str(s) => Ok(s),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn deserialize_bytes(&self, v: &mut Vec<u8>) -> DeserializeResult<Vec<u8>> {
        if DataTypePrefix::from(v.remove(0)) != DataTypePrefix::Str {
            return Err(DeserializeError::new(
                0,
                fn_name!(),
                switch_lang!(
                    "japanese" => "バイト列の読み込みに失敗しました",
                    "english" => "failed to load bytes",
                ),
            ));
        }
        let len = Self::deserialize_u32(v);
        Ok(v.drain(0..len as usize).collect())
    }
}
