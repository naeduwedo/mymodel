// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Threading;
using System.Threading.Tasks;
using Gguf.V1;
using GgufAiEngine.Shell.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace GgufAiEngine.Shell;

/// <summary>
/// The shell window: Overview and Settings, both rendered from the core's
/// snapshot (spec §4.1, §4.10).
/// </summary>
/// <remarks>
/// The window never invents a value. When the core is unreachable it says so and
/// keeps the last known snapshot on screen, labelled as stale, instead of showing
/// zeros that read like measurements.
/// </remarks>
public sealed partial class MainWindow : Window
{
    private static readonly TimeSpan ConnectTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(2);

    private readonly string _dataRoot;
    private readonly CancellationTokenSource _lifetime = new();
    private CoreClient? _client;
    private UiSnapshot? _lastSnapshot;

    public MainWindow()
    {
        InitializeComponent();
        Title = "GGUF AI Engine";

        // Same rule as the core: an explicit value wins, otherwise the platform
        // location. The environment variable lets a developer point the shell at
        // a scratch data directory without touching their real one.
        _dataRoot = Environment.GetEnvironmentVariable("GGUF_AI_ENGINE_DATA_DIR")
            ?? System.IO.Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "GGUF-AI-Engine");

        _ = RunAsync(_lifetime.Token);
    }

    private async Task RunAsync(CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            try
            {
                await RefreshOnceAsync(token);
            }
            catch (OperationCanceledException)
            {
                return;
            }
            catch (Exception ex)
            {
                // The shell must not crash because the core restarted.
                ShowDisconnected(ex.Message);
            }

            try
            {
                await Task.Delay(PollInterval, token);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    private async Task RefreshOnceAsync(CancellationToken token)
    {
        if (_client is null)
        {
            var endpoint = Endpoint.For(_dataRoot);
            _client = await CoreClient.TryConnectAsync(endpoint, ConnectTimeout, token);
            if (_client is null)
            {
                ShowDisconnected("No core is listening. Start it with: gguf-core --data-dir <DIR>");
                return;
            }
        }

        UiSnapshot snapshot;
        try
        {
            snapshot = await _client.GetSnapshotAsync(token);
        }
        catch (Grpc.Core.RpcException)
        {
            // The core went away between polls: drop the channel so the next
            // iteration reconnects instead of retrying a dead one forever.
            _client.Dispose();
            _client = null;
            ShowDisconnected("Lost the connection to the core. Retrying...");
            return;
        }

        _lastSnapshot = snapshot;
        Render(snapshot);
    }

    private void Render(UiSnapshot snapshot)
    {
        var overview = SnapshotMapper.ToOverview(snapshot);
        var settings = SnapshotMapper.ToSettings(snapshot);

        ConnectionBar.IsOpen = false;
        OverviewTimestamp.Text = $"Snapshot taken {overview.GeneratedAt}";
        TileList.ItemsSource = overview.Tiles;
        FeatureList.ItemsSource = overview.Features;

        SettingsSource.Text = settings.SourceLine;
        SettingGroups.ItemsSource = settings.Groups;
        SettingsIssueBar.IsOpen = settings.Issues.Count > 0;
        SettingsIssueBar.Title = "Configuration findings";
        SettingsIssueBar.Message = string.Join("\n", settings.Issues);

        StatusLine.Text = overview.AttentionRequired
            ? "Some capabilities are not implemented yet; see the Capabilities list."
            : "Everything the core currently supports is working.";
    }

    private void ShowDisconnected(string message)
    {
        ConnectionBar.Severity = InfoBarSeverity.Error;
        ConnectionBar.Title = "Core not connected";
        ConnectionBar.Message = message;
        ConnectionBar.IsOpen = true;

        StatusLine.Text = _lastSnapshot is null
            ? "No snapshot yet."
            : $"Showing the last snapshot from {_lastSnapshot.GeneratedAtLocal} (stale).";
    }

    private void OnNavigationChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        var tag = (args.SelectedItemContainer as NavigationViewItem)?.Tag as string;
        var showSettings = string.Equals(tag, "settings", StringComparison.Ordinal);
        OverviewPanel.Visibility = showSettings ? Visibility.Collapsed : Visibility.Visible;
        SettingsPanel.Visibility = showSettings ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>Stop polling and release the channel when the window closes.</summary>
    public void Shutdown()
    {
        _lifetime.Cancel();
        _client?.Dispose();
    }
}
