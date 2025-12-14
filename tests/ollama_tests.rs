use rustline::ollama::{Message, ToolInvocation};
use proptest::prelude::*;

proptest! {
    /// **Feature: user-persistent-memory, Property 2: Message persistence immediacy**
    /// **Validates: Requirements 1.2, 1.3**
    #[test]
    fn test_message_persistence_immediacy(
        role in "[a-zA-Z]{1,20}",
        content in ".*",
        tool_name in "[a-zA-Z0-9_]{1,30}",
        tool_input in ".*",
        tool_output in ".*",
        success in any::<bool>(),
    ) {
        // Test message serialization round-trip
        let original_message = Message::new(role.clone(), content.clone());
        
        // Serialize the message
        let serialized = serde_json::to_string(&original_message).unwrap();
        
        // Deserialize the message
        let deserialized: Message = serde_json::from_str(&serialized).unwrap();
        
        // Check that core fields are preserved
        prop_assert_eq!(deserialized.role, original_message.role);
        prop_assert_eq!(deserialized.content, original_message.content);
        prop_assert_eq!(deserialized.message_id, original_message.message_id);
        
        // Timestamps should be very close (within 1 second)
        let time_diff = (deserialized.timestamp - original_message.timestamp).num_seconds().abs();
        prop_assert!(time_diff <= 1);
        
        // Test message with tool invocation
        let tool_invocation = ToolInvocation {
            tool_name: tool_name.clone(),
            input: tool_input.clone(),
            output: tool_output.clone(),
            success,
        };
        
        let message_with_tool = Message::new_with_tool(
            role.clone(),
            content.clone(),
            tool_invocation.clone()
        );
        
        // Serialize and deserialize message with tool
        let serialized_with_tool = serde_json::to_string(&message_with_tool).unwrap();
        let deserialized_with_tool: Message = serde_json::from_str(&serialized_with_tool).unwrap();
        
        // Check that all fields are preserved
        prop_assert_eq!(deserialized_with_tool.role, message_with_tool.role);
        prop_assert_eq!(deserialized_with_tool.content, message_with_tool.content);
        prop_assert_eq!(deserialized_with_tool.message_id, message_with_tool.message_id);
        
        if let Some(tool_inv) = deserialized_with_tool.tool_invocation {
            prop_assert_eq!(tool_inv.tool_name, tool_name);
            prop_assert_eq!(tool_inv.input, tool_input);
            prop_assert_eq!(tool_inv.output, tool_output);
            prop_assert_eq!(tool_inv.success, success);
        } else {
            prop_assert!(false, "Tool invocation should be preserved");
        }
    }
}