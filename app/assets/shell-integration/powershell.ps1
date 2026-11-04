# LumenTerm shell entegrasyonu (PowerShell) — OSC 133 prompt marking.
# $PROFILE dosyanıza ekleyin (örn. notepad $PROFILE):
#   if ($env:LUMENTERM_SHELL_INTEGRATION) { . "C:\path\to\powershell.ps1" }
#
# Not: PowerShell'in bash/zsh'teki preexec/precmd hook'larına birebir
# karşılığı yok; burada `prompt` fonksiyonu üzerinden en yakın yaklaşım
# uygulanıyor (komut çalışmaya başladığı an değil, bir önceki komutun
# bitişi + yeni prompt'un başlangıcı işaretleniyor).

function global:prompt {
    $exitCode = $LASTEXITCODE
    $esc = [char]27
    # Önceki komutun bitişi (OSC 133;D;<exit_code>)
    [Console]::Out.Write("$esc]133;D;$exitCode$esc\")
    # Yeni prompt başlangıcı (OSC 133;A)
    [Console]::Out.Write("$esc]133;A$esc\")
    $path = (Get-Location).Path
    # Kullanıcı girdisinin başlangıcı (OSC 133;B) — prompt metni yazılmadan
    # hemen önce, LumenTerm bu noktadan sonrasını "komut girişi" sayar.
    [Console]::Out.Write("$esc]133;B$esc\")
    return "PS $path> "
}

# Komutun gerçek başlangıcını (OSC 133;C) işaretlemek için PSReadLine'ın
# "kabul edildi" olayını kullanıyoruz; bu olmadan C işareti B ile aynı ana
# denk gelir, ki bu da kabul edilebilir bir yaklaşıktır.
try {
    Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
        $esc = [char]27
        [Console]::Out.Write("$esc]133;C$esc\")
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
} catch {
    # PSReadLine yoksa (ör. eski Windows PowerShell sürümleri) sessizce atla.
}
