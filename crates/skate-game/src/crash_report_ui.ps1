$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$path = $env:SKATE_REPORT_PATH
$report = [IO.File]::ReadAllText($path)
$form = New-Object Windows.Forms.Form
$form.Text = 'Skate 3 Rust Engine — diagnostic report'
$form.ClientSize = New-Object Drawing.Size(570, 310)
$form.StartPosition = 'CenterScreen'
$form.FormBorderStyle = 'FixedDialog'
$form.MaximizeBox = $false
$label = New-Object Windows.Forms.Label
$label.SetBounds(16, 14, 535, 55)
$label.Text = "The game stopped unexpectedly. A text report has been saved.`nReview it before sharing with the team. Nothing is uploaded."
$form.Controls.Add($label)
$preview = New-Object Windows.Forms.TextBox
$preview.SetBounds(16, 73, 535, 175)
$preview.Multiline = $true
$preview.ReadOnly = $true
$preview.ScrollBars = 'Both'
$preview.WordWrap = $false
$preview.Text = $report.Replace("`n", "`r`n")
$form.Controls.Add($preview)
$copy = New-Object Windows.Forms.Button
$copy.SetBounds(16, 264, 130, 30)
$copy.Text = 'Copy report'
$copy.Add_Click({
    try { [Windows.Forms.Clipboard]::SetDataObject($report, $true, 5, 150); $copy.Text = 'Copied' }
    catch { [Windows.Forms.MessageBox]::Show('Clipboard unavailable. Select the report text and press Ctrl+C, or open the saved file.') }
})
$form.Controls.Add($copy)
$folder = New-Object Windows.Forms.Button
$folder.SetBounds(158, 264, 130, 30)
$folder.Text = 'Open folder'
$folder.Add_Click({
    try { Start-Process explorer.exe -ArgumentList ('"' + [IO.Path]::GetDirectoryName($path) + '"') }
    catch { [Windows.Forms.MessageBox]::Show('Saved report: ' + $path) }
})
$form.Controls.Add($folder)
$close = New-Object Windows.Forms.Button
$close.SetBounds(421, 264, 130, 30)
$close.Text = 'Close'
$close.Add_Click({ $form.Close() })
$form.Controls.Add($close)
$form.CancelButton = $close
[void]$form.ShowDialog()
