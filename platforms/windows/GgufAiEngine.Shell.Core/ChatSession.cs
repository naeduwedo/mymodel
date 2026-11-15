// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;
using Gguf.V1;

namespace GgufAiEngine.Shell.Core;

/// <summary>
/// One streamed chat answer, accumulated as it arrives.
/// </summary>
/// <remarks>
/// This type exists so the streaming rules can be tested without a window, a
/// core, or a GPU. The rules it enforces are the ones a naive loop gets wrong:
///
/// * an empty delta is not a token and must not be appended;
/// * the final chunk carries <c>Done</c> and the progress, and must not be shown
///   as text;
/// * a cancelled stream keeps whatever arrived - throwing the partial answer away
///   loses work the user watched being generated.
/// </remarks>
public sealed class ChatSession
{
    private readonly IAsyncEnumerable<ChatChunk> _chunks;

    /// <summary>Start a session over a stream of chunks.</summary>
    public ChatSession(IAsyncEnumerable<ChatChunk> chunks)
    {
        _chunks = chunks ?? throw new ArgumentNullException(nameof(chunks));
    }

    /// <summary>Everything received so far.</summary>
    public string Answer { get; private set; } = string.Empty;

    /// <summary>How many non-empty deltas arrived.</summary>
    public int Chunks { get; private set; }

    /// <summary>The final progress report, once the stream has ended.</summary>
    public TaskProgress? FinalProgress { get; private set; }

    /// <summary>Whether the stream reached its <c>Done</c> chunk.</summary>
    public bool Completed { get; private set; }

    /// <summary>Whether the caller cancelled the stream.</summary>
    public bool Cancelled { get; private set; }

    /// <summary>
    /// Yield each delta as it arrives, accumulating <see cref="Answer"/>.
    /// </summary>
    /// <remarks>
    /// Cancellation is handled here rather than left to the caller so that the
    /// partial answer survives: the loop stops, <see cref="Cancelled"/> is set,
    /// and whatever was generated remains readable.
    /// </remarks>
    public async IAsyncEnumerable<string> RunAsync(
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        await foreach (var chunk in _chunks.WithCancellation(cancellationToken).ConfigureAwait(false))
        {
            if (cancellationToken.IsCancellationRequested)
            {
                Cancelled = true;
                yield break;
            }

            if (chunk.Done)
            {
                Completed = true;
                FinalProgress = chunk.Progress;
                yield break;
            }

            // An empty delta is the runtime saying nothing, not a token.
            if (chunk.Delta.Length == 0)
            {
                continue;
            }

            Chunks++;
            Answer += chunk.Delta;
            yield return chunk.Delta;
        }
    }
}
