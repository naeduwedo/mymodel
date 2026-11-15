// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using Gguf.V1;
using GgufAiEngine.Shell.Core;
using Xunit;

namespace GgufAiEngine.Shell.Core.Tests;

/// <summary>
/// Streaming rules. These run without a core, a window or a GPU, which is the
/// point: the accumulation logic is where the subtle bugs live.
/// </summary>
public class ChatSessionTests
{
    /// <summary>Yield the given chunks, then stop.</summary>
    private static async IAsyncEnumerable<ChatChunk> Stream(
        IEnumerable<ChatChunk> chunks,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        foreach (var chunk in chunks)
        {
            cancellationToken.ThrowIfCancellationRequested();
            await Task.Yield();
            yield return chunk;
        }
    }

    private static ChatChunk Delta(string text) => new() { Delta = text, Done = false };

    private static ChatChunk Done(uint tokens = 3, ulong elapsed = 42) => new()
    {
        Delta = string.Empty,
        Done = true,
        Progress = new TaskProgress
        {
            TokensGenerated = tokens,
            ElapsedMs = elapsed,
            TokensPerSecond = 71.4,
        },
    };

    private static async Task<(List<string> deltas, ChatSession session)> RunAsync(
        IEnumerable<ChatChunk> chunks,
        CancellationToken cancellationToken = default)
    {
        var session = new ChatSession(Stream(chunks, cancellationToken));
        var deltas = new List<string>();
        await foreach (var delta in session.RunAsync(cancellationToken))
        {
            deltas.Add(delta);
        }

        return (deltas, session);
    }

    [Fact]
    public async Task Deltas_are_yielded_in_order_and_accumulated()
    {
        var (deltas, session) = await RunAsync(new[]
        {
            Delta("Once"),
            Delta(" upon"),
            Delta(" a time"),
            Done(),
        });

        Assert.Equal(new[] { "Once", " upon", " a time" }, deltas);
        Assert.Equal("Once upon a time", session.Answer);
        Assert.Equal(3, session.Chunks);
        Assert.True(session.Completed);
    }

    [Fact]
    public async Task An_empty_delta_is_not_a_token()
    {
        // The runtime sends empty deltas as keep-alives; counting them would make
        // the token count wrong and append nothing visible.
        var (deltas, session) = await RunAsync(new[]
        {
            Delta(""),
            Delta("real"),
            Delta(""),
            Done(),
        });

        Assert.Single(deltas);
        Assert.Equal(1, session.Chunks);
        Assert.Equal("real", session.Answer);
    }

    [Fact]
    public async Task The_done_chunk_carries_the_progress_and_is_not_text()
    {
        var (deltas, session) = await RunAsync(new[] { Delta("hi"), Done(tokens: 9, elapsed: 100) });

        Assert.Equal(new[] { "hi" }, deltas);
        Assert.NotNull(session.FinalProgress);
        Assert.Equal(9u, session.FinalProgress!.TokensGenerated);
        Assert.Equal(100ul, session.FinalProgress.ElapsedMs);
    }

    [Fact]
    public async Task A_stream_that_ends_without_done_is_not_completed()
    {
        var (_, session) = await RunAsync(new[] { Delta("partial") });

        Assert.False(session.Completed);
        Assert.Equal("partial", session.Answer);
    }

    [Fact]
    public async Task Cancelling_keeps_the_partial_answer()
    {
        // A user who stops a long answer must not lose what they already read.
        using var cts = new CancellationTokenSource();
        var session = new ChatSession(Stream(new[]
        {
            Delta("first "),
            Delta("second"),
            Delta("third"),
        }, cts.Token));

        var seen = new List<string>();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(async () =>
        {
            await foreach (var delta in session.RunAsync(cts.Token))
            {
                seen.Add(delta);
                if (seen.Count == 1)
                {
                    cts.Cancel();
                }
            }
        });

        Assert.Equal("first ", session.Answer);
    }

    [Fact]
    public async Task An_error_mid_stream_keeps_what_arrived()
    {
        static async IAsyncEnumerable<ChatChunk> Failing()
        {
            yield return Delta("ok so far");
            await Task.Yield();
            throw new InvalidOperationException("the worker died");
        }

        var session = new ChatSession(Failing());
        var seen = new List<string>();

        var error = await Assert.ThrowsAsync<InvalidOperationException>(async () =>
        {
            await foreach (var delta in session.RunAsync())
            {
                seen.Add(delta);
            }
        });

        Assert.Contains("worker died", error.Message);
        Assert.Equal("ok so far", session.Answer);
    }

    [Fact]
    public void A_null_stream_is_refused_immediately() =>
        Assert.Throws<ArgumentNullException>(() => new ChatSession(null!));
}

/// <summary>The conversation and the request built from it.</summary>
public class ChatTranscriptTests
{
    [Fact]
    public void An_empty_prompt_is_refused_before_it_reaches_the_core()
    {
        var transcript = new ChatTranscript();
        Assert.Throws<ArgumentException>(() => transcript.AddUser("   "));
        Assert.Throws<ArgumentException>(
            () => transcript.BuildRequest("m", string.Empty, 16, 0.7f));
    }

    [Fact]
    public void A_missing_model_is_reported_as_choose_a_model()
    {
        var transcript = new ChatTranscript();
        var error = Assert.Throws<ArgumentException>(
            () => transcript.BuildRequest("", "hello", 16, 0.7f));
        Assert.Contains("Choose a model", error.Message);
    }

    [Fact]
    public void The_request_carries_history_then_the_new_prompt()
    {
        var transcript = new ChatTranscript();
        transcript.AddUser("first question");
        transcript.AddAssistant("first answer");

        var request = transcript.BuildRequest("model-x", "second question", 64, 0.2f);

        Assert.Equal("model-x", request.ModelId);
        Assert.Equal(64u, request.MaxTokens);
        Assert.Equal(0.2f, request.Temperature);
        Assert.True(request.Stream);
        Assert.Equal(3, request.Messages.Count);
        Assert.Equal("user", request.Messages[0].Role);
        Assert.Equal("assistant", request.Messages[1].Role);
        Assert.Equal("second question", request.Messages[2].Content);
    }

    [Fact]
    public void A_system_prompt_goes_first()
    {
        var transcript = new ChatTranscript { SystemPrompt = "be brief" };
        transcript.AddUser("hi");

        var request = transcript.BuildRequest("m", "again", 16, 0.7f);

        Assert.Equal("system", request.Messages[0].Role);
        Assert.Equal("be brief", request.Messages[0].Content);
    }

    [Fact]
    public void Building_a_request_does_not_append_the_prompt_to_the_transcript()
    {
        // A failed call must not leave the transcript claiming a turn that never
        // happened, so the pending prompt is added to the request only.
        var transcript = new ChatTranscript();
        transcript.AddUser("stored");

        _ = transcript.BuildRequest("m", "pending", 16, 0.7f);

        Assert.Single(transcript.Messages);
        Assert.Equal("stored", transcript.Messages[0].Content);
    }

    [Fact]
    public void Clear_forgets_the_conversation()
    {
        var transcript = new ChatTranscript();
        transcript.AddUser("a");
        transcript.AddAssistant("b");
        transcript.Clear();
        Assert.Empty(transcript.Messages);
    }
}
