# rust-based-llm-cli 
# A Rust-Based Local AI Agent CLI

## Motivation

Developers often want an AI assistant that is **fast, private, and always available**, even when the internet is not. Most AI agents today—such as **LangChain**, **AutoGen**, and **LlamaIndex**—are implemented in **Python** or **JavaScript**, designed primarily for cloud APIs or web applications. These ecosystems are powerful but also **heavyweight**, often requiring virtual environments, external dependencies, and network access for basic tasks.

In contrast, **Rust** offers **speed, safety, and portability**. Rust binaries are self-contained, run natively on all platforms, and start instantly—perfect for building **lightweight, offline AI tools** that don’t need to depend on remote services. However, the Rust ecosystem currently lacks robust **local-first LLM agent frameworks**. While projects like [**aichat**](https://github.com/sigoden/aichat) demonstrate that Rust can drive conversational CLIs, they rely on remote APIs (OpenAI, Claude, etc.).

Our motivation is to explore what a **fully local, Rust-native LLM agent** can look like.  
We aim to show that Rust is not only capable of serving as an AI interface language but can also form the foundation of an **efficient, secure, and extensible local agent runtime**—connecting directly to **Ollama** for model inference without using the cloud.

This project, named **rust-based-llm-cli**, seeks to fill this gap. It combines:
- A **local-only AI workflow** (no cloud APIs)
- **Ollama-based inference** via local CLI or daemon
- **Context-aware session memory**
- **Agentic tool execution**
- And a clean **Ratatui terminal interface**

The result is a **lightweight LLM CLI** that has privacy, portability, and the system-level efficiency Rust is known for.

---

## Objective and Key Features

### Objective
Build a **Rust-based local LLM agent CLI** that connects directly to a locally running **Ollama** instance (through CLI or HTTP), enabling offline AI chat, tool execution, and contextual memory within a terminal-based interface.

The project’s primary goal is to **demonstrate the feasibility and design of local Rust agents**, using modern language features (async/await, traits, serde) and Rust’s concurrency model to implement an efficient agent runtime.

---

### Key Features

#### 1. Local Model Inference
- Fully offline operation through **Ollama** integration.  
- Compatible with popular local models such as **Llama 3** and **Qwen2.5**.  
- No dependency on remote servers or API tokens.

#### 2. Context-Aware Sessions
- Local session memory stored as JSONL files.  
- Multiple named sessions for different tasks or projects.  

#### 3. Terminal User Interface (Ratatui)
- Interactive and responsive terminal UI built with **Ratatui**.  
- Real-time streaming of model responses. 

#### 4. Agentic Tool System
- Implements a **ReAct-style reasoning loop**: the LLM can decide when to call tools, receive observations, and continue reasoning.  
- Configurable tool list ensures safe tool execution.  

#### 5. Optional MCP Integration
- Prototype integration for the **Model Context Protocol (MCP)**.  
- Demonstrates how Rust agents can communicate with external services.

---

## Tentative Plan

The team will complete the project over several weeks, with each member focusing on a clear component of the system.

### Team Roles and Responsibilities

| Member | Role | Responsibilities |
|---------|------|------------------|
| **Yiming Liu** | Ollama integration | Develop connectors for both CLI and local HTTP modes; handle token streaming and error recovery. |
| **Jinhua Yan** | UI & user experience | Build the Ratatui interface, manage real-time token display, and design session controls. |
| **Jiayan Xu** | Tool system & reasoning | Implement the tool registry, ReAct-style reasoning loop, and built-in tools (file system, HTTP, shell). |

---

### Implementation Phases

#### Phase 1 — Core Prototype 
- Build minimal Ollama CLI connector for streaming output.  
- Establish basic single-session chat functionality.

#### Phase 2 — UI & Session System
- Integrate **Ratatui** to create a live, interactive interface.  
- Add session management (create, list, rename, switch).  
- Display streaming responses and maintain local message logs.

#### Phase 3 — Agentic Tool Framework
- Define the modular tool registry and basic built-in tools.  
- Implement the reasoning–action–observation loop for tool use.  
- Display tool calls and outputs in the UI.

#### Phase 4 — HTTP Connector & Configuration
- Add optional HTTP-based connector to interact with local Ollama daemon.  
- Implement tool list configuration, error messages, and safety prompts.

#### Phase 5 — MCP Integration
- Implement basic Model Context Protocol (MCP) client.  
- Demonstrate editor or tool interoperability.

---

## Feasibility and Deliverables

### Feasibility
This project is practical and achievable within the course timeline:
- Relies only on **stable Rust crates** such as `ratatui`.  
- **Ollama** provides a unified interface for local inference, removing the need for complex model handling.  
- The team scope is well-defined, with independent tasks.  
- The technical challenges are balanced: real-time streaming, file I/O, UI rendering, and tools integration.

### Expected Deliverables
- A fully functional **local LLM agent CLI** capable of offline inference.  
- Clean and documented **Ratatui interface** with real-time response streaming.  
- Implementation of **tool-calling agentic workflow**.  
- Comprehensive documentation (setup, usage, and architecture overview).  

---

## Group Members

| Name | Student Number | Role | Contribution Focus  |
|------|----------------|------|---------------------|
| **Yiming Liu** |1011337402| Integration Developer | Project coordination, Ollama integration, and architecture design. |
| **Jinhua Yan** |1012858686| Interface Developer | Ratatui user interface, input handling, and session visualization. |
| **Jiayan Xu** |1012882436| Tool System Engineer | Tool registry design, ReAct reasoning loop, and safety control implementation. |