// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using Gguf.V1;
using GgufAiEngine.Shell.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

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

    private readonly ChatTranscript _transcript = new();
    private readonly List<ChatTurn> _turns = new();
    private CancellationTokenSource? _chatCancellation;
    private ChatTurn? _streamingTurn;

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
        var showChat = string.Equals(tag, "chat", StringComparison.Ordinal);
        var showCreate = string.Equals(tag, "create", StringComparison.Ordinal);
        var showSettings = string.Equals(tag, "settings", StringComparison.Ordinal);
        var showOverview = !showChat && !showCreate && !showSettings;

        OverviewPanel.Visibility = showOverview ? Visibility.Visible : Visibility.Collapsed;
        ChatPanel.Visibility = showChat ? Visibility.Visible : Visibility.Collapsed;
        CreatePanel.Visibility = showCreate ? Visibility.Visible : Visibility.Collapsed;
        SettingsPanel.Visibility = showSettings ? Visibility.Visible : Visibility.Collapsed;

        if (showChat)
        {
            PromptBox.Focus(FocusState.Programmatic);
        }
    }

    // ---------------------------------------------------------------- chat ----

    private void OnModelChanged(object sender, TextChangedEventArgs e)
    {
        SendButton.IsEnabled = !string.IsNullOrWhiteSpace(ModelBox.Text);
    }

    private void OnPromptKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == Windows.System.VirtualKey.Enter)
        {
            e.Handled = true;
            OnSendClicked(sender, new RoutedEventArgs());
        }
    }

    private void OnStopClicked(object sender, RoutedEventArgs e)
    {
        // Cancelling keeps the partial answer: the user watched those tokens
        // arrive and losing them would be a surprise.
        _chatCancellation?.Cancel();
    }

    private async void OnSendClicked(object sender, RoutedEventArgs e)
    {
        if (_client is null)
        {
            ShowChatMessage(InfoBarSeverity.Warning, "Not connected", "Start the core first.");
            return;
        }

        ChatRequest request;
        try
        {
            request = _transcript.BuildRequest(
                ModelBox.Text,
                PromptBox.Text,
                (uint)MaxTokensBox.Value,
                0.7f);
        }
        catch (ArgumentException ex)
        {
            ShowChatMessage(InfoBarSeverity.Warning, "Nothing to send", ex.Message);
            return;
        }

        PromptBox.Text = string.Empty;
        SetChatBusy(true);
        ChatBar.IsOpen = false;

        _transcript.AddUser(request.Messages[request.Messages.Count - 1].Content);
        var userTurn = new ChatTurn("You", _transcript.Messages[_transcript.Messages.Count - 1].Content);
        AddTurn(userTurn);

        var answer = new ChatTurn("Assistant", string.Empty);
        _streamingTurn = answer;
        AddTurn(answer);

        _chatCancellation = CancellationTokenSource.CreateLinkedTokenSource(_lifetime.Token);
        var session = new ChatSession(_client.ChatAsync(request, _chatCancellation.Token));

        try
        {
            await foreach (var delta in session.RunAsync(_chatCancellation.Token))
            {
                answer.Text += delta;
                RefreshTurns();
                TranscriptScroller.ChangeView(null, TranscriptScroller.ScrollableHeight, null);
            }

            _transcript.AddAssistant(session.Answer);
            ChatStatus.Text = session.Completed
                ? $"{session.Chunks} tokens in {session.FinalProgress?.ElapsedMs ?? 0} ms"
                : "The answer ended early.";
        }
        catch (OperationCanceledException)
        {
            _transcript.AddAssistant(session.Answer);
            ChatStatus.Text = $"Stopped after {session.Chunks} tokens.";
        }
        catch (Exception ex)
        {
            // Keep whatever arrived, and say what went wrong. The transcript keeps
            // the partial answer so the next turn still has context.
            _transcript.AddAssistant(session.Answer);
            ShowChatMessage(InfoBarSeverity.Error, "Chat failed", ex.Message);
            ChatStatus.Text = $"Failed after {session.Chunks} tokens.";
        }
        finally
        {
            _streamingTurn = null;
            _chatCancellation?.Dispose();
            _chatCancellation = null;
            SetChatBusy(false);
        }
    }

    // ------------------------------------------------------------- create ----

    private async void OnGenerateImageClicked(object sender, RoutedEventArgs e)
    {
        if (_client is null)
        {
            ShowCreateMessage(InfoBarSeverity.Warning, "Not connected", "Start the core first.");
            return;
        }

        var prompt = ImagePromptBox.Text;
        if (string.IsNullOrWhiteSpace(prompt))
        {
            ShowCreateMessage(InfoBarSeverity.Warning, "Nothing to draw", "Describe the image first.");
            return;
        }

        GenerateImageButton.IsEnabled = false;
        ImageStatus.Text = "Generating... this takes seconds.";
        CreateBar.IsOpen = false;

        try
        {
            var request = new ImageRequest
            {
                Prompt = prompt,
                Width = (uint)ImageWidthBox.Value,
                Height = (uint)ImageHeightBox.Value,
            };

            // A diffusion render is tens of seconds; the default deadline would
            // abandon a working call.
            using var deadline = CancellationTokenSource.CreateLinkedTokenSource(_lifetime.Token);
            deadline.CancelAfter(TimeSpan.FromMinutes(10));

            var png = await _client.GenerateImageAsync(request, deadline.Token);

            var path = Path.Combine(Path.GetTempPath(), $"gguf-image-{DateTime.Now:HHmmss}.png");
            await File.WriteAllBytesAsync(path, png);

            ImageStatus.Text = $"Wrote {png.Length / 1024} KiB to {path}";
        }
        catch (Exception ex)
        {
            // A missing runtime is FAILED_PRECONDITION with the reason the probe
            // produced, so showing the message verbatim is the useful thing to do.
            ImageStatus.Text = "Failed.";
            ShowCreateMessage(InfoBarSeverity.Error, "Image generation failed", ex.Message);
        }
        finally
        {
            GenerateImageButton.IsEnabled = true;
        }
    }

    private async void OnSpeakClicked(object sender, RoutedEventArgs e)
    {
        if (_client is null)
        {
            ShowCreateMessage(InfoBarSeverity.Warning, "Not connected", "Start the core first.");
            return;
        }

        var text = SpeechTextBox.Text;
        if (string.IsNullOrWhiteSpace(text))
        {
            ShowCreateMessage(InfoBarSeverity.Warning, "Nothing to say", "Enter some text first.");
            return;
        }

        SpeakButton.IsEnabled = false;
        SpeechStatus.Text = "Synthesising...";
        CreateBar.IsOpen = false;

        try
        {
            var wav = await _client.SynthesiseAsync(text, _lifetime.Token);
            var path = Path.Combine(Path.GetTempPath(), $"gguf-speech-{DateTime.Now:HHmmss}.wav");
            await File.WriteAllBytesAsync(path, wav);
            SpeechStatus.Text = $"Wrote {wav.Length / 1024} KiB to {path}";
        }
        catch (Exception ex)
        {
            SpeechStatus.Text = "Failed.";
            ShowCreateMessage(InfoBarSeverity.Error, "Speech synthesis failed", ex.Message);
        }
        finally
        {
            SpeakButton.IsEnabled = true;
        }
    }

    private void ShowCreateMessage(InfoBarSeverity severity, string title, string message)
    {
        CreateBar.Severity = severity;
        CreateBar.Title = title;
        CreateBar.Message = message;
        CreateBar.IsOpen = true;
    }

    private void SetChatBusy(bool busy)
    {
        SendButton.IsEnabled = !busy && !string.IsNullOrWhiteSpace(ModelBox.Text);
        StopButton.IsEnabled = busy;
        PromptBox.IsEnabled = !busy;
    }

    private void ShowChatMessage(InfoBarSeverity severity, string title, string message)
    {
        ChatBar.Severity = severity;
        ChatBar.Title = title;
        ChatBar.Message = message;
        ChatBar.IsOpen = true;
    }

    private void AddTurn(ChatTurn turn)
    {
        _turns.Add(turn);
        RefreshTurns();
    }

    private void RefreshTurns()
    {
        // Rebind so the streaming turn re-renders as its text grows. The list is
        // small (a conversation), so replacing the source is cheaper than
        // per-item change notification.
        TranscriptList.ItemsSource = null;
        TranscriptList.ItemsSource = _turns;
    }

    /// <summary>Stop polling and release the channel when the window closes.</summary>
    public void Shutdown()
    {
        _lifetime.Cancel();
        _client?.Dispose();
    }
}
/// <summary>One rendered turn in the chat pane.</summary>
/// <remarks>
/// Plain mutable properties: the pane rebinds its item source as tokens arrive,
/// which re-reads them. A per-item change notification would be more efficient
/// for very long conversations, but a conversation is small and rebinding keeps
/// the streaming path obvious.
/// </remarks>
public sealed class ChatTurn
{
    /// <summary>Create a turn.</summary>
    public ChatTurn(string speaker, string text)
    {
        Speaker = speaker;
        Text = text;
    }

    /// <summary>Who said it ("You" or "Assistant").</summary>
    public string Speaker { get; }

    /// <summary>The text so far.</summary>
    public string Text { get; set; }
}