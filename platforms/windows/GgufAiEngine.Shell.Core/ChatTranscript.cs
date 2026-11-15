// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Collections.Generic;
using System.Linq;
using Gguf.V1;

namespace GgufAiEngine.Shell.Core;

/// <summary>
/// The conversation shown in the chat pane, and the request built from it.
/// </summary>
/// <remarks>
/// Kept separate from the window so the rules are testable: an empty prompt is
/// refused here rather than sent to the core, which would otherwise load a model
/// to answer nothing.
/// </remarks>
public sealed class ChatTranscript
{
    private readonly List<ChatMessage> _messages = new();

    /// <summary>Messages in order, system first when present.</summary>
    public IReadOnlyList<ChatMessage> Messages => _messages;

    /// <summary>Optional system prompt applied to the next request.</summary>
    public string? SystemPrompt { get; set; }

    /// <summary>Append a user message.</summary>
    /// <exception cref="ArgumentException">The prompt is blank.</exception>
    public ChatMessage AddUser(string prompt)
    {
        if (string.IsNullOrWhiteSpace(prompt))
        {
            throw new ArgumentException("The prompt must not be empty.", nameof(prompt));
        }

        var message = new ChatMessage { Role = "user", Content = prompt };
        _messages.Add(message);
        return message;
    }

    /// <summary>Append an assistant message.</summary>
    public ChatMessage AddAssistant(string content)
    {
        var message = new ChatMessage
        {
            Role = "assistant",
            Content = content ?? string.Empty,
        };
        _messages.Add(message);
        return message;
    }

    /// <summary>Forget the conversation.</summary>
    public void Clear() => _messages.Clear();

    /// <summary>
    /// Build a request. <paramref name="prompt"/> is appended as a user message
    /// in the returned request without being stored, so a failed call does not
    /// leave the transcript claiming a turn that never happened.
    /// </summary>
    public ChatRequest BuildRequest(
        string modelId,
        string prompt,
        uint maxTokens,
        float temperature)
    {
        if (string.IsNullOrWhiteSpace(modelId))
        {
            throw new ArgumentException("Choose a model first.", nameof(modelId));
        }

        if (string.IsNullOrWhiteSpace(prompt))
        {
            throw new ArgumentException("The prompt must not be empty.", nameof(prompt));
        }

        var request = new ChatRequest
        {
            ModelId = modelId,
            MaxTokens = maxTokens,
            Temperature = temperature,
            Stream = true,
        };

        if (!string.IsNullOrWhiteSpace(SystemPrompt))
        {
            request.Messages.Add(new ChatMessage { Role = "system", Content = SystemPrompt });
        }

        request.Messages.AddRange(_messages.Select(message => message.Clone()));
        request.Messages.Add(new ChatMessage { Role = "user", Content = prompt });
        return request;
    }
}
