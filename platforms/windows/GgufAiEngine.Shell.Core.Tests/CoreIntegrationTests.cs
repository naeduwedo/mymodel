// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using GgufAiEngine.Shell.Core;
using Xunit;

namespace GgufAiEngine.Shell.Core.Tests;

/// <summary>
/// End-to-end test of the shell's client against the real core daemon, over the
/// real Windows named pipe.
/// </summary>
/// <remarks>
/// This is the test that catches the mistakes unit tests cannot: a DACL that
/// locks the shell out, a pipe name the core does not use, a gRPC service the
/// core stopped registering, or a snapshot field the shell reads differently.
///
/// It needs the `gguf-core` binary. CI builds it first; a developer runs
/// <c>cargo build -p gguf-engine-daemon</c>. When the binary is missing the test
/// is skipped rather than failed, and it says so — a silently passing test would
/// be worse than no test.
/// </remarks>
public class CoreIntegrationTests : IDisposable
{
    private readonly string _dataRoot;
    private readonly Process? _core;

    public CoreIntegrationTests()
    {
        _dataRoot = Path.Combine(Path.GetTempPath(), "gguf-shell-it-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(_dataRoot);

        var core = FindCoreBinary();
        if (core is not null)
        {
            _core = Process.Start(new ProcessStartInfo(core)
            {
                Arguments = $"--data-dir \"{_dataRoot}\"",
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            });
            WaitUntilListening(TimeSpan.FromSeconds(20));
        }
    }

    private static string? FindCoreBinary()
    {
        var explicitPath = Environment.GetEnvironmentVariable("GGUF_CORE_BIN");
        if (!string.IsNullOrEmpty(explicitPath) && File.Exists(explicitPath))
        {
            return explicitPath;
        }

        // Walk up from the test assembly to the repository root, then look in the
        // usual cargo output locations.
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            foreach (var profile in new[] { "debug", "release" })
            {
                var candidate = Path.Combine(dir.FullName, "target", profile, "gguf-core.exe");
                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }

            dir = dir.Parent;
        }

        return null;
    }

    private void WaitUntilListening(TimeSpan timeout)
    {
        var endpoint = Endpoint.For(_dataRoot);
        var deadline = DateTime.UtcNow + timeout;
        while (DateTime.UtcNow < deadline)
        {
            var client = CoreClient.TryConnectAsync(endpoint, TimeSpan.FromSeconds(1))
                .GetAwaiter().GetResult();
            if (client is not null)
            {
                client.Dispose();
                return;
            }

            Thread.Sleep(150);
        }

        throw new InvalidOperationException(
            $"core did not start listening on {endpoint}. Is the daemon built?");
    }

    [Fact]
    public async Task Shell_reads_a_live_snapshot_over_the_real_pipe()
    {
        RequireCore();

        var client = await CoreClient.TryConnectAsync(Endpoint.For(_dataRoot), TimeSpan.FromSeconds(5));
        Assert.NotNull(client);

        using (client)
        {
            var snapshot = await client!.GetSnapshotAsync();

            Assert.Equal(1u, snapshot.SnapshotVersion);
            Assert.NotNull(snapshot.Overview);
            Assert.NotNull(snapshot.Settings);

            var view = SnapshotMapper.ToOverview(snapshot);
            Assert.Contains(view.Tiles, t => t.Id == "core.state" && t.Value == "ready");
            Assert.Contains(view.Tiles, t => t.Id == "db.schema");
            Assert.Contains(view.Features, f => f.Id == "ipc" && f.StatusText == "supported");

            // The honesty rule, checked across the language boundary: anything
            // not built yet must arrive as planned *with a reason*.
            var planned = view.Features.Where(f => f.StatusText == "planned").ToList();
            Assert.NotEmpty(planned);
            Assert.All(planned, f => Assert.False(string.IsNullOrWhiteSpace(f.Reason)));

            var settings = SnapshotMapper.ToSettings(snapshot);
            Assert.NotEmpty(settings.Groups);
            Assert.All(settings.Groups.SelectMany(g => g.Rows), r => Assert.False(r.Editable));
        }
    }

    [Fact]
    public async Task Client_reports_null_when_no_core_is_running()
    {
        var emptyRoot = Path.Combine(Path.GetTempPath(), "gguf-nocore-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(emptyRoot);

        var client = await CoreClient.TryConnectAsync(Endpoint.For(emptyRoot), TimeSpan.FromSeconds(2));
        Assert.Null(client);
    }

    [Fact]
    public async Task A_second_core_on_the_same_data_root_is_refused_while_the_first_serves()
    {
        RequireCore();

        var core = FindCoreBinary()!;
        var second = Process.Start(new ProcessStartInfo(core)
        {
            Arguments = $"--data-dir \"{_dataRoot}\"",
            UseShellExecute = false,
            RedirectStandardError = true,
            RedirectStandardOutput = true,
            CreateNoWindow = true,
        })!;
        await second.WaitForExitAsync();
        Assert.Equal(3, second.ExitCode);

        var client = await CoreClient.TryConnectAsync(Endpoint.For(_dataRoot), TimeSpan.FromSeconds(5));
        Assert.NotNull(client);
        client!.Dispose();
    }

    /// <summary>
    /// Fail with an actionable message when the daemon binary is missing.
    /// </summary>
    /// <remarks>
    /// Deliberately not "skip": a silently skipped end-to-end test is worse than
    /// no test, because the suite then reports success while nothing crossed a
    /// pipe. Set GGUF_SKIP_CORE_INTEGRATION=1 to opt out explicitly.
    /// </remarks>
    private void RequireCore()
    {
        if (Environment.GetEnvironmentVariable("GGUF_SKIP_CORE_INTEGRATION") == "1")
        {
            Assert.Fail("skipped by GGUF_SKIP_CORE_INTEGRATION=1 (no end-to-end coverage in this run)");
        }

        Assert.True(
            _core is not null,
            "gguf-core binary not found. Run 'cargo build -p gguf-engine-daemon' and retry, " +
            "or set GGUF_CORE_BIN to its full path.");
    }

    /// <inheritdoc />
    public void Dispose()
    {
        if (_core is { HasExited: false })
        {
            try
            {
                _core.Kill(entireProcessTree: true);
            }
            catch (InvalidOperationException)
            {
                // Already gone.
            }
        }

        _core?.Dispose();
        try
        {
            Directory.Delete(_dataRoot, recursive: true);
        }
        catch (IOException)
        {
            // A leftover temp directory is not a test failure.
        }
    }
}

/// <summary>Minimal skip support (xunit v2 has no built-in Skip on a fact).</summary>
internal static class Skip
{
    public static void If(bool condition, string reason)
    {
        if (condition)
        {
            throw new SkipException(reason);
        }
    }
}

/// <summary>Thrown to mark a test as skipped.</summary>
public sealed class SkipException(string reason) : Exception(reason);