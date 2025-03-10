use cargo_faasta::github_oauth;

#[tokio::test]
async fn test_github_oauth_flow() {
    // Enable test mode
    github_oauth::enable_test_mode("test_user".to_string(), "test_token".to_string());
    
    // Call the OAuth flow function
    let result = github_oauth::github_oauth_flow().await;
    
    // Check the result
    assert!(result.is_ok());
    let (username, token) = result.unwrap();
    assert_eq!(username, "test_user");
    assert_eq!(token, "Bearer test_token");
}
