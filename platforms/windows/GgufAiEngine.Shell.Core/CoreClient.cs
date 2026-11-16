// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Threading;
using System.Threading.Tasks;
using Gguf.V1;
using Grpc.Core;
using Grpc.Net.Client;

namespace GgufAiEngine.Shell.Core;

/// <summary>
/// Connects to the core over its local IPC transport (spec §3.2).
/// </summary>
/// <remarks>
/// Two properties matter and a naive client gets both wrong:
///
/// * the transport is a named pipe, never TCP — the shell must not open a
///   network port even if a firewall would allow it;
/// * "no core is running" is an ordinary state. <see cref="TryConnectAsync"/>
///   returns <c>null</c> instead of throwing, so the window can show a
///   reconnect affordance rather than an error dialog.
/// </remarks>
public sealed class CoreClient : IDisposable
{
    private readonly GrpcChannel _channel;

    private CoreClient(GrpcChannel channel, string endpoint)
    {
        _channel = channel;
        Endpoint = endpoint;
    }

    /// <summary>The endpoint this client talks to.</summary>
    public string Endpoint { get; }

    /// <summary>
    /// Connect, or return <c>null</c> when nothing is listening.
    /// </summary>
    public static async Task<CoreClient?> TryConnectAsync(
        string endpoint,
        TimeSpan timeout,
        CancellationToken cancellationToken = default)
    {
        var channel = GrpcChannel.ForAddress(
            "http://localhost",
            new GrpcChannelOptions
            {
                // No TLS: the peer is this user on this machine, and the pipe's
                // DACL already decided whether the connection may exist.
                HttpHandler = new SocketsHttpHandler
                {
                    ConnectCallback = async (_, ct) =>
                    {
                        var pipe = CreatePipe(endpoint);
                        try
                        {
                            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
                            cts.CancelAfter(timeout);
                            await pipe.ConnectAsync(cts.Token).ConfigureAwait(false);
                            return pipe;
                        }
                        catch
                        {
                            await pipe.DisposeAsync().ConfigureAwait(false);
                            throw;
                        }
                    },
                },
            });

        var client = new CoreClient(channel, endpoint);
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            cts.CancelAfter(timeout);
            var health = new HealthService.HealthServiceClient(channel);
            await health.PingAsync(new Empty(), cancellationToken: cts.Token).ConfigureAwait(false);
            return client;
        }
        catch (RpcException)
        {
            client.Dispose();
            return null;
        }
        catch (IOException)
        {
            client.Dispose();
            return null;
        }
        catch (TimeoutException)
        {
            client.Dispose();
            return null;
        }
    }

    /// <summary>Open the underlying pipe stream for an endpoint.</summary>
    private static NamedPipeClientStream CreatePipe(string endpoint)
    {
        var server = ".";
        var name = endpoint;

        if (endpoint.StartsWith(Shell.Core.Endpoint.PipePrefix, StringComparison.OrdinalIgnoreCase))
        {
            // Strip \\.\pipe\ to get the pipe name.
            var remainder = endpoint[Shell.Core.Endpoint.PipePrefix.Length..];
            var slash = remainder.IndexOf('\\');
            if (slash >= 0)
            {
                server = remainder[..slash];
                name = remainder[(slash + 1)..];
            }
            else
            {
                name = remainder;
            }
        }

        // The core's pipe requires message-mode writes to be flushed; WriteThrough
        // keeps small gRPC frames from sitting in the client buffer.
        return new NamedPipeClientStream(
            server,
            name,
            PipeDirection.InOut,
            PipeOptions.Asynchronous | PipeOptions.WriteThrough);
    }

    /// <summary>Fetch the Overview + Settings snapshot.</summary>
    public async Task<UiSnapshot> GetSnapshotAsync(CancellationToken cancellationToken = default)
    {
        var ui = new UiService.UiServiceClient(_channel);
        return await ui.GetSnapshotAsync(new UiSnapshotRequest(), cancellationToken: cancellationToken)
            .ConfigureAwait(false);
    }

    /// <summary>Health report (used by the status strip).</summary>
    public async Task<HealthReport> GetHealthAsync(CancellationToken cancellationToken = default)
    {
        var health = new HealthService.HealthServiceClient(_channel);
        return await health.GetHealthAsync(new HealthRequest(), cancellationToken: cancellationToken)
            .ConfigureAwait(false);
    }

    /// <summary>
    /// Stream a chat completion.
    /// </summary>
    /// <remarks>
    /// Returned as <see cref="IAsyncEnumerable{T}"/> so the caller can render each
    /// token as it arrives. Cancelling the enumeration tells the core to stop; the
    /// core unloads the model when the stream ends, including on a disconnect, so
    /// abandoning the stream does not leak a worker process.
    /// </remarks>
    public IAsyncEnumerable<ChatChunk> ChatAsync(
        ChatRequest request,
        CancellationToken cancellationToken = default)
    {
        var inference = new InferenceService.InferenceServiceClient(_channel);
        return inference.Chat(request, cancellationToken: cancellationToken)
            .ResponseStream
            .ReadAllAsync(cancellationToken);
    }

    /// <summary>Generate an image, returning PNG bytes.</summary>
    /// <remarks>
    /// The call blocks for the whole render: a 512x512 image takes seconds, and a
    /// streaming progress channel would be more moving parts than the wait is
    /// worth. The cancellation token still lets the window give up.
    /// </remarks>
    public async Task<byte[]> GenerateImageAsync(
        ImageRequest request,
        CancellationToken cancellationToken = default)
    {
        var media = new MediaService.MediaServiceClient(_channel);
        var reply = await media
            .GenerateImageAsync(request, cancellationToken: cancellationToken)
            .ConfigureAwait(false);
        return reply.Png.ToByteArray();
    }

    /// <summary>Generate a video clip, yielding progress as the runtime advances.</summary>
    /// <remarks>
    /// The stream ends after exactly one event carrying the clip, or one carrying
    /// an error. Callers should set a deadline that reflects minutes, not seconds;
    /// the default gRPC deadline would abandon a working render.
    /// </remarks>
    public IAsyncEnumerable<VideoProgressEvent> GenerateVideoAsync(
        VideoRequest request,
        CancellationToken cancellationToken = default)
    {
        var media = new MediaService.MediaServiceClient(_channel);
        return media
            .GenerateVideo(request, cancellationToken: cancellationToken)
            .ResponseStream
            .ReadAllAsync(cancellationToken);
    }

    /// <summary>Synthesise speech, returning WAV bytes.</summary>
    public async Task<byte[]> SynthesiseAsync(
        string text,
        CancellationToken cancellationToken = default)
    {
        var media = new MediaService.MediaServiceClient(_channel);
        var reply = await media
            .SynthesiseAsync(new SpeechRequest { Text = text }, cancellationToken: cancellationToken)
            .ConfigureAwait(false);
        return reply.Wav.ToByteArray();
    }

    /// <summary>Generate music or a sound effect, returning WAV bytes.</summary>
    /// <remarks>
    /// Blocks for the render: seconds to a couple of minutes. The runtime is a
    /// one-shot CLI, so there is no progress channel to subscribe to.
    /// </remarks>
    public async Task<MusicReply> GenerateMusicAsync(
        string prompt,
        uint durationSeconds,
        CancellationToken cancellationToken = default)
    {
        var media = new MediaService.MediaServiceClient(_channel);
        return await media
            .GenerateMusicAsync(
                new MusicRequest { Prompt = prompt, DurationSeconds = durationSeconds },
                cancellationToken: cancellationToken)
            .ConfigureAwait(false);
    }

    /// <summary>Transcribe an audio file.</summary>
    public async Task<string> TranscribeAsync(
        byte[] audio,
        string filename,
        string? language = null,
        CancellationToken cancellationToken = default)
    {
        var request = new TranscriptionRequest
        {
            Audio = Google.Protobuf.ByteString.CopyFrom(audio),
            Filename = filename,
        };
        if (!string.IsNullOrWhiteSpace(language))
        {
            request.Language = language;
        }

        var media = new MediaService.MediaServiceClient(_channel);
        var reply = await media
            .TranscribeAsync(request, cancellationToken: cancellationToken)
            .ConfigureAwait(false);
        return reply.Text;
    }

    /// <inheritdoc />
    public void Dispose() => _channel.Dispose();
}