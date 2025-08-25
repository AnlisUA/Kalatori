use crate::error::{NotHexError, UtilError};

pub mod logger;
pub mod shutdown;
pub mod task_tracker;

pub fn unhex(hex_data: &str, what_is_hex: NotHexError) -> Result<Vec<u8>, UtilError> {
    if let Some(stripped) = hex_data.strip_prefix("0x") {
        const_hex::decode(stripped).map_err(|_| UtilError::NotHex(what_is_hex))
    } else {
        const_hex::decode(hex_data).map_err(|_| UtilError::NotHex(what_is_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unhex_valid_hex_without_prefix() {
        let result = unhex("48656c6c6f", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello");
    }

    #[test]
    fn test_unhex_valid_hex_with_prefix() {
        let result = unhex("0x48656c6c6f", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello");
    }

    #[test]
    fn test_unhex_empty_string() {
        let result = unhex("", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_unhex_empty_string_with_prefix() {
        let result = unhex("0x", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_unhex_single_byte() {
        let result = unhex("41", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"A");
    }

    #[test]
    fn test_unhex_single_byte_with_prefix() {
        let result = unhex("0x41", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"A");
    }

    #[test]
    fn test_unhex_multiple_bytes() {
        let result = unhex("48656c6c6f20576f726c64", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello World");
    }

    #[test]
    fn test_unhex_multiple_bytes_with_prefix() {
        let result = unhex("0x48656c6c6f20576f726c64", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello World");
    }

    #[test]
    fn test_unhex_with_zeros() {
        let result = unhex("000102030405", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_unhex_with_zeros_with_prefix() {
        let result = unhex("0x000102030405", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_unhex_invalid_hex_odd_length() {
        let result = unhex("123", NotHexError::BlockHash);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UtilError::NotHex(NotHexError::BlockHash)));
    }

    #[test]
    fn test_unhex_invalid_hex_odd_length_with_prefix() {
        let result = unhex("0x123", NotHexError::BlockHash);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UtilError::NotHex(NotHexError::BlockHash)));
    }

    #[test]
    fn test_unhex_invalid_hex_characters() {
        let result = unhex("48656c6c6g", NotHexError::BlockHash);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UtilError::NotHex(NotHexError::BlockHash)));
    }

    #[test]
    fn test_unhex_invalid_hex_characters_with_prefix() {
        let result = unhex("0x48656c6c6g", NotHexError::BlockHash);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UtilError::NotHex(NotHexError::BlockHash)));
    }

    #[test]
    fn test_unhex_invalid_hex_mixed_case() {
        let result = unhex("48656C6C6F", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello");
    }

    #[test]
    fn test_unhex_invalid_hex_mixed_case_with_prefix() {
        let result = unhex("0x48656C6C6F", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello");
    }

    #[test]
    fn test_unhex_different_error_types() {
        // Test with different NotHexError variants
        let block_hash_result = unhex("invalid", NotHexError::BlockHash);
        assert!(matches!(block_hash_result.unwrap_err(), UtilError::NotHex(NotHexError::BlockHash)));

        let extrinsic_result = unhex("invalid", NotHexError::Extrinsic);
        assert!(matches!(extrinsic_result.unwrap_err(), UtilError::NotHex(NotHexError::Extrinsic)));

        let metadata_result = unhex("invalid", NotHexError::Metadata);
        assert!(matches!(metadata_result.unwrap_err(), UtilError::NotHex(NotHexError::Metadata)));

        let storage_key_result = unhex("invalid", NotHexError::StorageKey);
        assert!(matches!(storage_key_result.unwrap_err(), UtilError::NotHex(NotHexError::StorageKey)));

        let storage_value_result = unhex("invalid", NotHexError::StorageValue);
        assert!(matches!(storage_value_result.unwrap_err(), UtilError::NotHex(NotHexError::StorageValue)));
    }

    #[test]
    fn test_unhex_large_hex_string() {
        // Test with a larger hex string (32 bytes)
        let large_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = unhex(large_hex, NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 32);
    }

    #[test]
    fn test_unhex_large_hex_string_with_prefix() {
        // Test with a larger hex string with prefix (32 bytes)
        let large_hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = unhex(large_hex, NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 32);
    }

    #[test]
    fn test_unhex_only_prefix() {
        // Test with just "0x" prefix
        let result = unhex("0x", NotHexError::BlockHash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_unhex_prefix_without_hex() {
        // Test with "0x" followed by invalid hex
        let result = unhex("0xinvalid", NotHexError::BlockHash);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UtilError::NotHex(NotHexError::BlockHash)));
    }
}
