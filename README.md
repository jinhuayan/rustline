# Rustline: A Rust-Based Local LLM Agent CLI

## 1. Team Information

| Name       | Student Number | Preferred Email                                                   |
| ---------- | -------------- | ----------------------------------------------------------------- |
| Yiming Liu | 1011337402     | yimingpaul.liu@mail.utoronto.ca |
| Jinhua Yan | 1012858686     | jinhua.yan@mail.utoronto.ca |
| Jiayan Xu  | 1012882436     | jiayan.xu@mail.utoronto.ca  |

---

## 2. Motivation

Large Language Model (LLM) agents have become a common productivity tool for developers. However, most existing agent frameworks—such as LangChain, AutoGen, and LlamaIndex—are implemented in Python or JavaScript and are primarily designed for cloud-based APIs. These solutions often introduce significant overhead, including virtual environments, heavyweight runtime dependencies, and reliance on external services.

Beyond performance considerations, cloud-based LLM agents raise important **privacy and data ownership concerns**. User prompts, conversation history, and intermediate reasoning steps are typically transmitted to third-party services, making it difficult to guarantee data confidentiality. In contrast, **local LLM deployment** ensures that all data—including prompts, session history, and model inference—remains entirely on the user’s machine. By combining local model inference with on-disk session storage, Rustline provides strong privacy guarantees without sacrificing functionality.

In addition to privacy, this project is motivated by an **exploratory engineering goal**: investigating whether modern agent paradigms commonly used in Python ecosystems can be effectively implemented in **Rust**. Agent patterns such as **ReAct (Reasoning + Acting)** are well-established in Python-based frameworks, but are far less explored in Rust. This project demonstrates how Rust’s async runtime, strong type system, and modular design can be used to implement agentic reasoning loops, tool execution, and persistent context management in a systems programming language.

By addressing both privacy concerns and architectural experimentation, Rustline aims to fill a gap in the Rust ecosystem and demonstrate that Rust can serve as a first-class platform for building efficient, extensible, and fully local LLM agents.

---

## 3. Objectives

The primary objective of this project is to design and implement a **Rust-based local LLM agent CLI** that operates entirely on the user’s machine by connecting to a locally running **Ollama** instance. Specifically, the project aims to:

* Enable fully offline LLM interaction without cloud APIs or API keys
* Support context-aware conversations through persistent local sessions
* Implement an agentic workflow that allows the LLM to invoke tools
* Provide an interactive and responsive terminal user interface
* Serve as an educational reference for building AI agents in Rust

---

## 4. Key Features

This section describes the core features implemented in Rustline and explains how each feature contributes directly to the project objectives of enabling a local-first, extensible, and agentic LLM experience in Rust.

### 4.1 Local Model Inference via Ollama

Rustline integrates directly with a locally running **Ollama** instance, enabling LLM inference without any reliance on cloud-based APIs or external network services. The system supports both CLI-based invocation and local HTTP communication with Ollama.

Key characteristics:

* Fully offline operation once models are downloaded
* No API keys, authentication, or network latency
* Compatibility with popular open-source models such as **gemma3**
* Easy for changing the model base on local hardware capability
* Token-level streaming to the UI for responsive, low-latency interaction

This design ensures privacy, reproducibility, and performance, aligning with Rustline’s goal of being a local-first AI agent.

### 4.2 Context-Aware Session Management

Rustline implements persistent session management to maintain conversational context across interactions. Each session represents an independent conversation thread and is stored locally using lightweight JSONL files.

Key characteristics:

* Multiple named sessions for different tasks or projects
* Automatic loading and saving of session history
* Clear separation of contexts to avoid cross-session contamination
* No dependency on databases or external storage systems

This feature allows users to treat Rustline as a long-running assistant rather than a stateless chat interface.

### 4.3 Interactive Terminal User Interface (Ratatui)

The project provides a full-screen terminal user interface built using **Ratatui**, enabling an interactive and visually structured experience entirely within the command line.

Key characteristics:

* Real-time streaming display of model responses
* Dedicated UI regions for user input, model output, and system messages
* Responsive rendering loop suitable for asynchronous workloads
* Keyboard-driven interaction optimized for developer workflows

The UI demonstrates how Rust can be used to build modern, responsive TUIs that integrate seamlessly with asynchronous AI systems.

### 4.4 Agentic Tool Execution Framework

Rustline implements an agentic workflow inspired by the **ReAct (Reasoning + Acting)** pattern. In this loop, the LLM is prompted to reason about a task, decide whether a tool invocation is necessary, observe the tool’s output, and continue reasoning until a final answer is produced.

Key characteristics:

* Explicit tool registry with controlled execution
* Structured tool input and output handling
* Clear separation between reasoning steps and actions
* Safe execution boundaries to prevent unintended side effects

This framework enables Rustline to go beyond simple chat and act as a functional AI agent capable of interacting with the local environment.

### 4.5 Modular and Extensible Architecture

Rustline is designed with modularity as a core principle. Major components—such as the model connector, session manager, agent logic, and tool system—are implemented as independent modules with clear interfaces. This design makes the system easy to understand, test, and extend.

Key characteristics:

* Separation of concerns between UI, agent logic, model inference, and persistence
* Trait-based abstractions for model connectors and tools
* Clear module boundaries that support future extension (e.g., new models or tools)

This architectural approach reinforces Rustline’s role as both a practical application and an educational reference for Rust-based AI systems.

---

## 5. Project Structure and Architecture

This section describes the overall structure of the Rustline codebase and explains how its components interact at runtime.

### 5.1 High-Level Architecture

At a high level, Rustline consists of four major layers:

1. **Terminal UI Layer** – Handles user input and displays model responses using Ratatui
2. **Agent Layer** – Manages reasoning, tool invocation, and control flow
3. **Model Connector Layer** – Interfaces with Ollama for local LLM inference
4. **Persistence Layer** – Stores session history and state on disk

User input flows from the UI layer to the agent, which decides whether to call the model or invoke a tool. Model outputs and tool observations are then streamed back to the UI in real time.

---

### 5.2 ReAct-Style Agent Loop

Rustline implements an agent loop inspired by the **ReAct (Reasoning + Acting)** paradigm. Rather than treating the LLM as a simple text generator, the agent alternates between reasoning steps and concrete actions.

The loop operates as follows:

1. The user submits a prompt through the terminal UI
2. The agent constructs a prompt that includes conversation history and available tools
3. The LLM generates a reasoning step and may request a tool invocation
4. If a tool is requested, the agent executes the tool and captures its output
5. The tool output is fed back into the LLM as an observation
6. The loop continues until the LLM produces a final response

This design enables the LLM to solve multi-step tasks and interact with the local environment in a controlled and explainable manner.

---

### 5.3 Source Code Structure

The Rustline repository is organized into logical modules, each responsible for a specific aspect of the system:

* `src/main.rs`
  Entry point of the application. Initializes the runtime, loads configuration, and starts the terminal UI.

* `src/app.rs`
  Coordinates high-level application state and connects the UI with the agent logic.

* `src/ui.rs`
  Contains all Ratatui-based UI components, including layout, rendering, and input handling.

* `src/agent.rs`
  Implements the agent logic, including the ReAct-style reasoning loop and decision-making process.

* `src/ollama`
  Provides the abstraction layer for LLM inference and the concrete Ollama connector implementation.

* `src/tools.rs`
  Defines the tool registry and individual tools that can be invoked by the agent.

* `src/persistence/`
  Handles persistent session storage, loading, and saving using local JSONL files.

* `tests/`
  Tests for checking each file's functionality works well.
  

This structure reflects a clean separation of responsibilities and makes the codebase approachable for both users and developers.

---

## 6. User / Developer Guide

### 6.1 Prerequisites

The following software is required:

* Rust (version 1.91.1)
* Cargo (version 1.91.1)
* Ollama (Download latest)


### 6.2 Install Ollama and Models

```bash
# Install Ollama following official instructions
# Then pull a model
ollama pull gemma3

# Start Ollama service by CLI or ollama run at back
ollama serve
```

### 6.3 Build Rustline

```bash
git clone https://github.com/jinhuayan/rustline
cd rustline
cargo build --release
```

### 6.4 Run Rustline

```bash
cargo run
```

Once running, users can enter prompts directly in the terminal interface. The agent will stream responses in real time and invoke tools when appropriate.

---

### 6.5 Reproducibility Guide

The project is fully reproducible on a clean Ubuntu or macOS system by following these steps exactly:

1. Install Rust and Cargo
2. Install and start Ollama
3. Pull a supported model (e.g., `gemma3`)
4. Clone the repository
5. Build the project using `cargo build --release`
6. Run the application using `cargo run`

No environment variables or external configuration files are required. All features—chatting, session management, and tool execution—can be tested using the default setup without additional clarification.

---

## 7. Individual Contributions

### Yiming Liu

* Overall system architecture and project coordination
* Ollama CLI and HTTP integration
* Streaming token handling and error recovery

### Jinhua Yan

* Terminal user interface using Ratatui
* User input handling and real-time display
* Session visualization and interaction flow

### Jiayan Xu

* Agent tool design
* ReAct-style reasoning loop implementation
* Safety controls and tool execution logic

---

## 9. Lessons Learned and Concluding Remarks

This project highlighted both the strengths and challenges of building AI systems in Rust. Implementing asynchronous streaming, managing terminal UI state, and designing safe tool execution required careful consideration of Rust’s ownership and concurrency model. At the same time, Rust’s performance, safety guarantees, and strong type system proved highly valuable for building a reliable agent runtime.

In conclusion, Rustline demonstrates that Rust is a good choice for building local-first LLM agents. The project fills a gap in the current Rust ecosystem and provides educational value for developers interested in systems-level AI tooling. Future work includes expanding the tool ecosystem, improving configuration flexibility, and supporting additional local inference backends.

---

## 10. Video Slide Presentation

Presentation URL HERE

---

## 11. Video Demo

Demo URL HERE
