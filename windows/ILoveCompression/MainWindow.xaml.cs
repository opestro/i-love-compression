using System.Collections.ObjectModel;
using System.Diagnostics;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using Microsoft.Win32;

namespace ILoveCompression;

public partial class MainWindow : Window
{
    private static readonly string[] VideoExtensions = [".mp4", ".mov", ".m4v", ".mkv", ".avi", ".webm", ".wmv", ".flv", ".mts", ".m2ts", ".ts"];
    private static readonly string[] ImageExtensions = [".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tif", ".tiff", ".ico"];

    // name, format, quality, resize box (null = keep size); "Custom" = whatever is set
    private static readonly (string Name, string Format, int Quality, (int, int)? Box)[] Presets =
    [
        ("Web Optimized", "webp", 75, (1920, 1080)),
        ("Social Media", "jpeg", 85, (1200, 1200)),
        ("High Quality", "png", 100, null),
        ("Smallest", "avif", 60, null),
    ];
    private static readonly (string Key, string Title, string Hint)[] Formats =
    [
        ("jpeg", "JPEG", "Photos. Opens everywhere."),
        ("png", "PNG", "Graphics and screenshots. 100 = lossless; lower values reduce colours (like TinyPNG), usually 60–80% smaller."),
        ("webp", "WebP", "Best all-rounder for websites. Every modern browser."),
        ("avif", "AVIF", "Smallest files. Modern browsers, iPhone, Android."),
        ("ico", "ICO", "Windows and website icons, 16–256 px."),
    ];

    private readonly ObservableCollection<Job> _jobs = [];
    private Hardware _hardware = new();
    private bool _isVideo;
    private bool _running;
    private bool _stopping;
    private bool _applyingPreset;
    private string? _outputFolder;
    private EngineRun? _current;

    public MainWindow()
    {
        InitializeComponent();
        JobList.ItemsSource = _jobs;
        foreach (var p in Presets) PresetBox.Items.Add(p.Name);
        PresetBox.Items.Add("Custom");
        foreach (var f in Formats) FormatBox.Items.Add(f.Title);
        PresetBox.SelectedIndex = 0;
        Loaded += OnLoaded;
    }

    /// Command line: ILoveCompression.exe [--compress] [--mode video|image] file…
    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        _hardware = await EngineRun.ProbeAsync();
        UpdateVideoHints();
        var args = Environment.GetCommandLineArgs().Skip(1).ToList();
        var modeIndex = args.IndexOf("--mode");
        if (modeIndex >= 0 && modeIndex + 1 < args.Count) ShowMode(args[modeIndex + 1] == "video");
        var files = args.Where(File.Exists).ToList();
        if (files.Count > 0)
        {
            ShowMode(IsVideoFile(files[0]));
            Add(files);
            if (args.Contains("--compress")) Start();
        }
    }

    private static bool IsVideoFile(string path) => VideoExtensions.Contains(Path.GetExtension(path).ToLowerInvariant());
    private static bool IsImageFile(string path) => ImageExtensions.Contains(Path.GetExtension(path).ToLowerInvariant());

    // ---- navigation

    private void OnChooseVideo(object sender, RoutedEventArgs e) => ShowMode(true);
    private void OnChooseImage(object sender, RoutedEventArgs e) => ShowMode(false);

    private void ShowMode(bool video)
    {
        if (_running) return;
        if (video != _isVideo) _jobs.Clear();
        _isVideo = video;
        HomeView.Visibility = Visibility.Collapsed;
        ModeView.Visibility = Visibility.Visible;
        ModeTitle.Text = video ? "Video" : "Image";
        VideoOptions.Visibility = video ? Visibility.Visible : Visibility.Collapsed;
        ImageOptions.Visibility = video ? Visibility.Collapsed : Visibility.Visible;
        DropGlyph.Text = video ? "" : "";
        DropTitle.Text = video ? "Drop videos here" : "Drop images here";
        UpdateVideoHints();
        Refresh();
    }

    private void OnHome(object sender, RoutedEventArgs e)
    {
        if (_running) return;
        _jobs.Clear();
        ModeView.Visibility = Visibility.Collapsed;
        HomeView.Visibility = Visibility.Visible;
    }

    // ---- files

    private void OnAddFiles(object sender, RoutedEventArgs e)
    {
        var exts = string.Join(";", (_isVideo ? VideoExtensions : ImageExtensions).Select(x => "*" + x));
        var dialog = new OpenFileDialog
        {
            Multiselect = true,
            Filter = $"{(_isVideo ? "Videos" : "Images")}|{exts}",
        };
        if (dialog.ShowDialog(this) == true) Add(dialog.FileNames);
    }

    private void OnDragOver(object sender, DragEventArgs e)
    {
        e.Effects = e.Data.GetDataPresent(DataFormats.FileDrop) && ModeView.Visibility == Visibility.Visible ? DragDropEffects.Copy : DragDropEffects.None;
        e.Handled = true;
    }

    private void OnDrop(object sender, DragEventArgs e)
    {
        if (e.Data.GetData(DataFormats.FileDrop) is not string[] paths) return;
        if (ModeView.Visibility != Visibility.Visible && paths.Length > 0) ShowMode(IsVideoFile(paths[0]));
        Add(paths);
    }

    private void Add(IEnumerable<string> paths)
    {
        var known = _jobs.Select(j => j.Path).ToHashSet(StringComparer.OrdinalIgnoreCase);
        foreach (var path in paths.Where(p => (_isVideo ? IsVideoFile(p) : IsImageFile(p)) && !known.Contains(p)))
        {
            _jobs.Add(new Job(path, _isVideo));
        }
        Refresh();
    }

    private void OnShow(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is Job { Output: { } output })
            Process.Start("explorer.exe", $"/select,\"{output}\"");
    }

    private void OnClearFinished(object sender, RoutedEventArgs e)
    {
        foreach (var job in _jobs.Where(j => j.Status is JobStatus.Done or JobStatus.Failed).ToList()) _jobs.Remove(job);
        Refresh();
    }

    // ---- running

    private void OnCompress(object sender, RoutedEventArgs e)
    {
        if (_running) { _stopping = true; _current?.Stop(); }
        else Start();
    }

    private void Start()
    {
        if (_running || !_jobs.Any(j => j.Status == JobStatus.Waiting)) return;
        _stopping = false;
        if (_isVideo) RunNextVideo(); else RunImages();
    }

    // Videos one at a time: each encode already fills the CPU or the GPU encoder.
    private void RunNextVideo()
    {
        var job = _stopping ? null : _jobs.FirstOrDefault(j => j.Status == JobStatus.Waiting);
        if (job == null) { Finish(); return; }
        job.SetRunning();
        var codec = CodecAv1.IsChecked == true ? "av1" : "hevc";
        var gpu = EnginePanel.Visibility == Visibility.Visible && EngineGpu.IsChecked == true && EngineGpu.IsEnabled;
        Run(["video", job.Path, "--codec", codec, "--engine", gpu ? "gpu" : "cpu"], e => job.Apply(e), () =>
        {
            if (job.Status == JobStatus.Running) job.SetFailed("Stopped");
            RunNextVideo();
        });
    }

    // Images in one engine run, which compresses a few at once.
    private void RunImages()
    {
        var batch = _jobs.Where(j => j.Status == JobStatus.Waiting).ToList();
        batch.ForEach(j => j.SetRunning());
        var args = new List<string>
        {
            "image", "--format", Formats[Math.Max(0, FormatBox.SelectedIndex)].Key, "--quality", ((int)QualitySlider.Value).ToString(),
            "--name", RenameBox.IsChecked == true ? PatternBox.Text : "{original_name}-compressed",
            "--number", int.TryParse(StartNumberBox.Text, out var n) ? n.ToString() : "1",
        };
        if (ResizeBox.IsChecked == true)
        {
            args.AddRange(["--width", WidthBox.Text.Trim(), "--height", HeightBox.Text.Trim()]);
            if (AspectBox.IsChecked != true) args.Add("--stretch");
        }
        if (MetadataBox.IsChecked == true) args.Add("--keep-metadata");
        if (_outputFolder != null) args.AddRange(["--out-dir", _outputFolder]);
        args.AddRange(batch.Select(j => j.Path));
        Run(args, e =>
        {
            if (e.Index is int i && i >= 0 && i < batch.Count) batch[i].Apply(e);
        }, () =>
        {
            foreach (var job in batch.Where(j => j.Status == JobStatus.Running)) job.SetWaiting(); // stopped before it started
            Finish();
        });
    }

    private void Run(IEnumerable<string> args, Action<EngineEvent> onEvent, Action onExit)
    {
        _running = true;
        var run = new EngineRun(args, e => { onEvent(e); Refresh(); }, onExit);
        _current = run;
        Refresh();
        try
        {
            run.Start();
        }
        catch (Exception ex)
        {
            foreach (var job in _jobs.Where(j => j.Status == JobStatus.Running)) job.SetFailed($"Couldn't start the engine: {ex.Message}");
            Finish();
        }
    }

    private void Finish()
    {
        _current = null;
        _running = false;
        Refresh();
    }

    private void Refresh()
    {
        var hasJobs = _jobs.Count > 0;
        DropZone.Visibility = hasJobs ? Visibility.Collapsed : Visibility.Visible;
        JobList.Visibility = hasJobs ? Visibility.Visible : Visibility.Collapsed;
        BottomBar.Visibility = hasJobs ? Visibility.Visible : Visibility.Collapsed;
        OptionsPane.IsEnabled = !_running;
        HomeButton.IsEnabled = !_running;
        ClearButton.IsEnabled = !_running;
        var waiting = _jobs.Count(j => j.Status == JobStatus.Waiting);
        CompressButton.Content = _running ? "Stop" : waiting == 1 ? "Compress 1 File" : $"Compress {waiting} Files";
        CompressButton.IsEnabled = _running || waiting > 0;
        var saved = _jobs.Where(j => j.Status == JobStatus.Done).Sum(j => Math.Max(0, j.Before - j.After));
        SavedLabel.Text = saved > 0 ? $"♥ Saved {Job.Bytes(saved)}" : "";
    }

    // ---- video options

    private void OnVideoOptionChanged(object sender, RoutedEventArgs e) => UpdateVideoHints();

    private void UpdateVideoHints()
    {
        if (EngineHint == null) return; // Checked fires while the XAML is still loading
        var av1 = CodecAv1.IsChecked == true;
        CodecHint.Text = av1
            ? "About 1.6× smaller than HEVC, but Apple devices only play it on M3+ Macs and iPhone 15 Pro+. Chrome, Firefox and VLC play it anywhere."
            : "Plays in Windows, QuickTime, Safari and on every iPhone and iPad.";
        var anyGpu = _hardware.NvencHevc || _hardware.NvencAv1;
        EnginePanel.Visibility = anyGpu ? Visibility.Visible : Visibility.Collapsed;
        var gpuCan = av1 ? _hardware.NvencAv1 : _hardware.NvencHevc;
        EngineGpu.IsEnabled = gpuCan;
        EngineHint.Text = !gpuCan
            ? "This NVIDIA GPU can't encode AV1 (that needs an RTX 40 series), so AV1 uses the CPU."
            : EngineGpu.IsChecked == true
                ? "Uses your NVIDIA GPU: about 3× faster, same size, visually the same quality."
                : "Uses the CPU: about 3× slower, slightly higher quality.";
    }

    // ---- image options

    private void OnPresetChanged(object sender, SelectionChangedEventArgs e)
    {
        if (PresetBox.SelectedIndex < 0 || PresetBox.SelectedIndex >= Presets.Length) return; // Custom
        var p = Presets[PresetBox.SelectedIndex];
        _applyingPreset = true;
        FormatBox.SelectedIndex = Array.FindIndex(Formats, f => f.Key == p.Format);
        QualitySlider.Value = p.Quality;
        ResizeBox.IsChecked = p.Box != null;
        if (p.Box is (int w, int h)) { WidthBox.Text = w.ToString(); HeightBox.Text = h.ToString(); }
        _applyingPreset = false;
        UpdateImageControls();
    }

    private void OnImageOptionChanged(object sender, RoutedEventArgs e) => MarkCustom();

    private void OnQualityChanged(object sender, RoutedPropertyChangedEventArgs<double> e) => MarkCustom();

    private void OnSizePreset(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not string tag) return;
        var parts = tag.Split('x');
        WidthBox.Text = parts[0];
        HeightBox.Text = parts[1];
    }

    private void MarkCustom()
    {
        if (PresetBox == null || FormatBox == null) return; // during InitializeComponent
        if (!_applyingPreset) PresetBox.SelectedIndex = Presets.Length; // "Custom"
        UpdateImageControls();
    }

    private void UpdateImageControls()
    {
        if (QualityValue == null || ResizePanel == null) return;
        var format = Formats[Math.Max(0, FormatBox.SelectedIndex)];
        QualityValue.Text = ((int)QualitySlider.Value).ToString();
        QualityRow.Visibility = format.Key == "ico" ? Visibility.Collapsed : Visibility.Visible;
        FormatHint.Text = format.Hint;
        ResizePanel.Visibility = ResizeBox.IsChecked == true ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnRenameToggled(object sender, RoutedEventArgs e) =>
        RenamePanel.Visibility = RenameBox.IsChecked == true ? Visibility.Visible : Visibility.Collapsed;

    private void OnChooseFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "Save compressed images to" };
        if (dialog.ShowDialog(this) != true) return;
        _outputFolder = dialog.FolderName;
        FolderLabel.Text = _outputFolder;
        ResetFolderButton.Visibility = Visibility.Visible;
    }

    private void OnResetFolder(object sender, RoutedEventArgs e)
    {
        _outputFolder = null;
        FolderLabel.Text = "Next to the originals";
        ResetFolderButton.Visibility = Visibility.Collapsed;
    }
}
