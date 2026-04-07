#[cfg(test)]
mod tests {
    use crate::auth::*;

    #[test]
    fn test_validate_rejects_invalid_length() {
        let user_address = "0x123";
        let message_hash = "0x456";
        let signature = vec!["0x789".to_string()]; // Only 1 element, should be 2

        let result = validate_signature_format(user_address, message_hash, &signature);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid signature length"));
    }

    #[test]
    fn test_validate_rejects_invalid_address_format() {
        let invalid_address = "not_a_hex_string";
        let message_hash = "0x456";
        let signature = vec!["0x789".to_string(), "0xABC".to_string()];

        let result = validate_signature_format(invalid_address, message_hash, &signature);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid user address"));
    }

    #[test]
    fn test_validate_rejects_invalid_message_hash() {
        let user_address = "0x0123456789abcdef0123456789abcdef01234567";
        let invalid_hash = "not_a_hash";
        let signature = vec!["0x789".to_string(), "0xABC".to_string()];

        let result = validate_signature_format(user_address, invalid_hash, &signature);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid message hash"));
    }

    #[test]
    fn test_validate_rejects_invalid_signature_r() {
        let user_address = "0x0123456789abcdef0123456789abcdef01234567";
        let message_hash = "0x0123456789abcdef0123456789abcdef01234567";
        let signature = vec!["invalid_r".to_string(), "0xABC".to_string()];

        let result = validate_signature_format(user_address, message_hash, &signature);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid signature element 0"));
    }

    #[test]
    fn test_extract_user_from_signature_returns_error() {
        let signature = vec!["0x789".to_string(), "0xABC".to_string()];

        let result = extract_user_from_signature(&signature);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot extract user from signature alone"));
    }

    // Integration test — requires a live Starknet Sepolia RPC and valid account + signature
    #[tokio::test]
    #[ignore]
    async fn test_verify_signature_with_valid_signature() {
        let rpc_url = "https://starknet-sepolia.public.blastapi.io/rpc/v0_7";
        let user_address = "0x03ee9e18edc71a6df30ac3aca2e0b02a198fbce19b7480a63a0d71cbd76652e0";
        let message_hash = "0x0397e76d1667c4454bfb83514e120583af836f8e32a516765497823eabe16a3f";
        let signature_r = "0x0411494b501a98abd8262b08414728b3f3bb4d00f2c3bfdaa31a21c3d6e59a";
        let signature_s = "0x07dd3a306bc659698d3d40fe327fdb95dfb8dfa7f7fef094fdfe55e1c2a87e3c";
        let signature = vec![signature_r.to_string(), signature_s.to_string()];

        let result = verify_signature(rpc_url, user_address, message_hash, &signature).await;

        match result {
            Ok(is_valid) => println!("Signature validation result: {}", is_valid),
            Err(e) => println!("Signature validation error: {}", e),
        }
    }
}
