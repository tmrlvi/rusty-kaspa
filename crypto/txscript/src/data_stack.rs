use core::fmt::Debug;
use crate::TxScriptError;
use core::mem::size_of;
use core::iter;

pub(crate) type Stack = Vec<Vec<u8>>;

pub(crate) trait DataStack {
    fn pop_item<const SIZE: usize, T: Debug>(&mut self) -> Result<[T; SIZE], TxScriptError>
        where
            Vec<u8>: OpcodeData<T>;
    fn last_item<const SIZE: usize, T: Debug>(&self) -> Result<[T; SIZE], TxScriptError>
        where
            Vec<u8>: OpcodeData<T>;
    fn pop_raw<const SIZE: usize>(&mut self) -> Result<[Vec<u8>; SIZE], TxScriptError>;
    fn last_raw<const SIZE: usize>(&self) -> Result<[Vec<u8>; SIZE], TxScriptError>;
    fn push_item<T: Debug>(&mut self, item: T)
        where
            Vec<u8>: OpcodeData<T>;
    fn drop_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError>;
    fn dup_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError>;
    fn over_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError>;
    fn rot_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError>;
    fn swap_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError>;
}

pub(crate) trait OpcodeData<T> {
    fn deserialize(&self) -> Result<T, TxScriptError>;
    fn serialize(from: &T) -> Self;
}

fn check_minimal_data_encoding(v: &[u8]) -> Result<(), TxScriptError> {
    if v.len() == 0 {
        return Ok(())
    }

    // Check that the number is encoded with the minimum possible
    // number of bytes.
    //
    // If the most-significant-byte - excluding the sign bit - is zero
    // then we're not minimal. Note how this test also rejects the
    // negative-zero encoding, [0x80].
    if v[v.len() - 1] & 0x7f == 0 {
        // One exception: if there's more than one byte and the most
        // significant bit of the second-most-significant-byte is set
        // it would conflict with the sign bit. An example of this case
        // is +-255, which encode to 0xff00 and 0xff80 respectively.
        // (big-endian).
        if v.len() == 1 || v[v.len() - 2] & 0x80 == 0 {
            return Err(TxScriptError::NotMinimalData(
                format!("numeric value encoded as {:x?} is not minimally encoded", v)
            ));
        }
    }

    return Ok(())
}

impl OpcodeData<i64> for Vec<u8> {
    #[inline]
    fn deserialize(&self) -> Result<i64, TxScriptError> {
        match self.len() {
            l if l > 4 => Err(TxScriptError::InvalidState(format!("numeric value encoded as {:x?} is {} bytes which exceeds the max allowed of {}", self, l, 4))),
            l if l > size_of::<i64>() => Err(TxScriptError::InvalidState(format!("numeric value encoded as {:x?} is longer than 8 bytes", self))),
            l if l == 0 => Ok(0),
            _ => {
                check_minimal_data_encoding(self)?;
                let msb = self[self.len() - 1];
                let sign = (1 - 2 * ((msb >> 7) as i64));
                let first_byte = ((msb & 0x7f) as i64);
                Ok(self[..self.len() - 1].iter().rev().map(|v| *v as i64)
                    .fold(first_byte, |accum, item| (accum << 8) + item)*sign)
            }
        }
    }

    #[inline]
    fn serialize(from: &i64) -> Self {
        let sign = from.signum();
        let mut positive = from.abs();
        let mut last_saturated = false;
        let mut number_vec: Vec<u8> = iter::from_fn(move || {
            if positive == 0 {
                if last_saturated {
                    last_saturated = false;
                    Some(0)
                } else {
                    None
                }
            } else {
                let value = positive & 0xff;
                last_saturated = (value & 0x80) != 0;
                positive >>= 8;
                Some(value as u8)
            }
        }).collect();
        if sign == -1 {
            match number_vec.last_mut() {
                Some(num) => *num |= 0x80,
                _ => unreachable!(),
            }
        }
        number_vec
    }
}


impl OpcodeData<i32> for Vec<u8> {
    #[inline]
    fn deserialize(&self) -> Result<i32, TxScriptError> {
        let res = OpcodeData::<i64>::deserialize(self)?;
        i32::try_from(res.clamp(i32::MIN as i64, i32::MAX as i64)).map_err(|e| TxScriptError::InvalidState(format!("data is too big for `i32`: {}", e)))
    }

    #[inline]
    fn serialize(from: &i32) -> Self {
        OpcodeData::<i64>::serialize(&(*from as i64))
    }
}

impl OpcodeData<bool> for Vec<u8> {
    #[inline]
    fn deserialize(&self) -> Result<bool, TxScriptError> {
        if self.len() == 0 {
            Ok(false)
        } else {
            // Negative 0 is also considered false
            Ok(self[self.len() - 1] & 0x7f != 0x0 || self[..self.len() - 1].iter().any(|&b| b != 0x0))
        }
    }

    #[inline]
    fn serialize(from: &bool) -> Self {
        match from {
            true => vec![1],
            false => vec![],
        }
    }
}

impl DataStack for Stack {
    #[inline]
    fn pop_item<const SIZE: usize, T: Debug>(&mut self) -> Result<[T; SIZE], TxScriptError>
        where
            Vec<u8>: OpcodeData<T>,
    {
        if self.len() < SIZE {
            return Err(TxScriptError::EmptyStack);
        }
        Ok(<[T; SIZE]>::try_from(self.split_off(self.len() - SIZE).iter().map(|v| v.deserialize()).collect::<Result<Vec<T>, _>>()?)
            .expect("Already exact item"))
    }

    #[inline]
    fn last_item<const SIZE: usize, T: Debug>(&self) -> Result<[T; SIZE], TxScriptError>
        where
            Vec<u8>: OpcodeData<T>,
    {
        if self.len() < SIZE {
            return Err(TxScriptError::EmptyStack);
        }
        Ok(<[T; SIZE]>::try_from(self[self.len() - SIZE..].iter().map(|v| v.deserialize()).collect::<Result<Vec<T>, _>>()?)
            .expect("Already exact item"))
    }

    #[inline]
    fn  pop_raw<const SIZE: usize>(&mut self) -> Result<[Vec<u8>; SIZE], TxScriptError> {
        if self.len() < SIZE {
            return Err(TxScriptError::EmptyStack);
        }
        Ok(<[Vec<u8>; SIZE]>::try_from(self.split_off(self.len() - SIZE)).expect("Already exact item"))
    }

    #[inline]
    fn last_raw<const SIZE: usize>(&self) -> Result<[Vec<u8>; SIZE], TxScriptError> {
        if self.len() < SIZE {
            return Err(TxScriptError::EmptyStack);
        }
        Ok(<[Vec<u8>; SIZE]>::try_from(self[self.len() - SIZE..].to_vec()).expect("Already exact item"))
    }

    #[inline]
    fn push_item<T: Debug>(&mut self, item: T)
        where
            Vec<u8>: OpcodeData<T>,
    {
        Vec::push(self, OpcodeData::serialize(&item));
    }

    #[inline]
    fn drop_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError> {
        match self.len() >= SIZE {
            true => {
                self.truncate(self.len() - SIZE);
                Ok(())
            }
            false => Err(TxScriptError::EmptyStack),
        }
    }

    #[inline]
    fn dup_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError> {
        match self.len() >= SIZE {
            true => {
                self.extend_from_slice(self.clone()[self.len() - SIZE..].iter().as_slice());
                Ok(())
            }
            false => Err(TxScriptError::EmptyStack),
        }
    }

    #[inline]
    fn over_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError> {
        match self.len() >= 2 * SIZE {
            true => {
                self.extend_from_slice(self.clone()[self.len() - 2 * SIZE..self.len() - SIZE].iter().as_slice());
                Ok(())
            }
            false => Err(TxScriptError::EmptyStack),
        }
    }

    #[inline]
    fn rot_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError> {
        match self.len() >= 3 * SIZE {
            true => {
                let drained = self.drain(self.len() - 3 * SIZE..self.len() - 2 * SIZE).collect::<Vec<Vec<u8>>>();
                self.extend(drained);
                Ok(())
            }
            false => Err(TxScriptError::EmptyStack),
        }
    }

    #[inline]
    fn swap_item<const SIZE: usize>(&mut self) -> Result<(), TxScriptError> {
        match self.len() >= 2 * SIZE {
            true => {
                let drained = self.drain(self.len() - 2 * SIZE..self.len() - SIZE).collect::<Vec<Vec<u8>>>();
                self.extend(drained);
                Ok(())
            }
            false => Err(TxScriptError::EmptyStack),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Error;
    use txscript_errors::TxScriptError;
    use super::OpcodeData;

    // TestScriptNumBytes
    #[test]
    fn test_serialize() {
        struct TestCase{num: i64, serialized: Vec<u8>}

        let tests = vec![
            TestCase{num: 0, serialized: vec![]},
            TestCase{num: 1, serialized: hex::decode("01").expect("failed parsing hex")},
            TestCase{num: -1, serialized: hex::decode("81").expect("failed parsing hex")},
            TestCase{num: 127, serialized: hex::decode("7f").expect("failed parsing hex")},
            TestCase{num: -127, serialized: hex::decode("ff").expect("failed parsing hex")},
            TestCase{num: 128, serialized: hex::decode("8000").expect("failed parsing hex")},
            TestCase{num: -128, serialized: hex::decode("8080").expect("failed parsing hex")},
            TestCase{num: 129, serialized: hex::decode("8100").expect("failed parsing hex")},
            TestCase{num: -129, serialized: hex::decode("8180").expect("failed parsing hex")},
            TestCase{num: 256, serialized: hex::decode("0001").expect("failed parsing hex")},
            TestCase{num: -256, serialized: hex::decode("0081").expect("failed parsing hex")},
            TestCase{num: 32767, serialized: hex::decode("ff7f").expect("failed parsing hex")},
            TestCase{num: -32767, serialized: hex::decode("ffff").expect("failed parsing hex")},
            TestCase{num: 32768, serialized: hex::decode("008000").expect("failed parsing hex")},
            TestCase{num: -32768, serialized: hex::decode("008080").expect("failed parsing hex")},
            TestCase{num: 65535, serialized: hex::decode("ffff00").expect("failed parsing hex")},
            TestCase{num: -65535, serialized: hex::decode("ffff80").expect("failed parsing hex")},
            TestCase{num: 524288, serialized: hex::decode("000008").expect("failed parsing hex")},
            TestCase{num: -524288, serialized: hex::decode("000088").expect("failed parsing hex")},
            TestCase{num: 7340032, serialized: hex::decode("000070").expect("failed parsing hex")},
            TestCase{num: -7340032, serialized: hex::decode("0000f0").expect("failed parsing hex")},
            TestCase{num: 8388608, serialized: hex::decode("00008000").expect("failed parsing hex")},
            TestCase{num: -8388608, serialized: hex::decode("00008080").expect("failed parsing hex")},
            TestCase{num: 2147483647, serialized: hex::decode("ffffff7f").expect("failed parsing hex")},
            TestCase{num: -2147483647, serialized: hex::decode("ffffffff").expect("failed parsing hex")},

            // Values that are out of range for data that is interpreted as
            // numbers, but are allowed as the result of numeric operations.
            TestCase{num: 2147483648, serialized: hex::decode("0000008000").expect("failed parsing hex")},
            TestCase{num: -2147483648, serialized: hex::decode("0000008080").expect("failed parsing hex")},
            TestCase{num: 2415919104, serialized: hex::decode("0000009000").expect("failed parsing hex")},
            TestCase{num: -2415919104, serialized: hex::decode("0000009080").expect("failed parsing hex")},
            TestCase{num: 4294967295, serialized: hex::decode("ffffffff00").expect("failed parsing hex")},
            TestCase{num: -4294967295, serialized: hex::decode("ffffffff80").expect("failed parsing hex")},
            TestCase{num: 4294967296, serialized: hex::decode("0000000001").expect("failed parsing hex")},
            TestCase{num: -4294967296, serialized: hex::decode("0000000081").expect("failed parsing hex")},
            TestCase{num: 281474976710655, serialized: hex::decode("ffffffffffff00").expect("failed parsing hex")},
            TestCase{num: -281474976710655, serialized: hex::decode("ffffffffffff80").expect("failed parsing hex")},
            TestCase{num: 72057594037927935, serialized: hex::decode("ffffffffffffff00").expect("failed parsing hex")},
            TestCase{num: -72057594037927935, serialized: hex::decode("ffffffffffffff80").expect("failed parsing hex")},
            TestCase{num: 9223372036854775807, serialized: hex::decode("ffffffffffffff7f").expect("failed parsing hex")},
            TestCase{num: -9223372036854775807, serialized: hex::decode("ffffffffffffffff").expect("failed parsing hex")},
        ];

        for test in tests {
            let serialized: Vec<u8> = OpcodeData::<i64>::serialize(&test.num);
            assert_eq!(serialized, test.serialized);
        }
    }

    // TestMakeScriptNum
    #[test]
    fn test_deserialize() {
        let default_script_num_len = 4;
        let err_num_too_big = Err(TxScriptError::NumberTooBig("".to_string()));
        let err_minimal_data = Err(TxScriptError::NotMinimalData("".to_string()));
        struct TestCase{serialized: Vec<u8>, num_len: usize, result: Result<i64, TxScriptError>}

        let tests = vec![
            TestCase{serialized: hex::decode("80").expect("failed parsing hex"), num_len: default_script_num_len, result: Err(TxScriptError::NotMinimalData("numeric value encoded as [80] is not minimally encoded".to_string())) },

            // Minimally encoded valid values with minimal encoding flag.
            // Should not error and return expected integral number.
            TestCase{serialized: vec![], num_len: default_script_num_len, result: Ok(0)},
            TestCase{serialized: hex::decode("01").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(1)},
            TestCase{serialized: hex::decode("81").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-1)},
            TestCase{serialized: hex::decode("7f").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(127)},
            TestCase{serialized: hex::decode("ff").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-127)},
            TestCase{serialized: hex::decode("8000").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(128)},
            TestCase{serialized: hex::decode("8080").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-128)},
            TestCase{serialized: hex::decode("8100").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(129)},
            TestCase{serialized: hex::decode("8180").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-129)},
            TestCase{serialized: hex::decode("0001").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(256)},
            TestCase{serialized: hex::decode("0081").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-256)},
            TestCase{serialized: hex::decode("ff7f").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(32767)},
            TestCase{serialized: hex::decode("ffff").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-32767)},
            TestCase{serialized: hex::decode("008000").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(32768)},
            TestCase{serialized: hex::decode("008080").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-32768)},
            TestCase{serialized: hex::decode("ffff00").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(65535)},
            TestCase{serialized: hex::decode("ffff80").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-65535)},
            TestCase{serialized: hex::decode("000008").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(524288)},
            TestCase{serialized: hex::decode("000088").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-524288)},
            TestCase{serialized: hex::decode("000070").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(7340032)},
            TestCase{serialized: hex::decode("0000f0").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-7340032)},
            TestCase{serialized: hex::decode("00008000").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(8388608)},
            TestCase{serialized: hex::decode("00008080").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-8388608)},
            TestCase{serialized: hex::decode("ffffff7f").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(2147483647)},
            TestCase{serialized: hex::decode("ffffffff").expect("failed parsing hex"), num_len: default_script_num_len, result: Ok(-2147483647)},
            TestCase{serialized: hex::decode("ffffffff7f").expect("failed parsing hex"), num_len: 5, result: Ok(549755813887)},
            TestCase{serialized: hex::decode("ffffffffff").expect("failed parsing hex"), num_len: 5, result: Ok(-549755813887)},
            TestCase{serialized: hex::decode("ffffffffffffff7f").expect("failed parsing hex"), num_len: 8, result: Ok(9223372036854775807)},
            TestCase{serialized: hex::decode("ffffffffffffffff").expect("failed parsing hex"), num_len: 8, result: Ok(-9223372036854775807)},

            // Minimally encoded values that are out of range for data that
            // is interpreted as script numbers with the minimal encoding
            // flag set. Should error and return 0.
            TestCase{serialized: hex::decode("0000008000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("0000008080").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("0000009000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("0000009080").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffff00").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffff80").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("0000000001").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("0000000081").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffffffff00").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffffffff80").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffffffffff00").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffffffffff80").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffffffffff7f").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("ffffffffffffffff").expect("failed parsing hex"), num_len: default_script_num_len, result: err_num_too_big.clone() },

            // Non-minimally encoded, but otherwise valid values with
            // minimal encoding flag. Should error and return 0.
            TestCase{serialized: hex::decode("00").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },       // 0
            TestCase{serialized: hex::decode("0100").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },     // 1
            TestCase{serialized: hex::decode("7f00").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },     // 127
            TestCase{serialized: hex::decode("800000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },   // 128
            TestCase{serialized: hex::decode("810000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },   // 129
            TestCase{serialized: hex::decode("000100").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },   // 256
            TestCase{serialized: hex::decode("ff7f00").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() },   // 32767
            TestCase{serialized: hex::decode("00800000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() }, // 32768
            TestCase{serialized: hex::decode("ffff0000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() }, // 65535
            TestCase{serialized: hex::decode("00000800").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() }, // 524288
            TestCase{serialized: hex::decode("00007000").expect("failed parsing hex"), num_len: default_script_num_len, result: err_minimal_data.clone() }, // 7340032
            TestCase{serialized: hex::decode("0009000100").expect("failed parsing hex"), num_len: 5, result: err_minimal_data.clone() },                 // 16779520
            // Values above 8 bytes should always return error
            TestCase{serialized: hex::decode("ffffffffffffffffff").expect("failed parsing hex"), num_len: 9, result: err_num_too_big.clone() },
            TestCase{serialized: hex::decode("00000000000000000000").expect("failed parsing hex"), num_len: 10, result: err_num_too_big.clone() },
        ];

        for test in tests {
            // Ensure the error code is of the expected type and the error
            // code matches the value specified in the test instance.
            assert_eq!(test.serialized.deserialize(), test.result);
        }
    }
}