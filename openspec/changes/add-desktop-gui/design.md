# Design

## Context

O OSX-KVM entrega shell scripts (`OpenCore-Boot.sh` com valores fixos no código), um downloader em Python (`fetch-macOS-v2.py`) e os binários de boot (OpenCore qcow2, OVMF). No host de referência (Ubuntu 24.04, Ryzen 9 9900X, QEMU 8.2.2, OSX-KVM `4c378a4`) o fluxo manual funcionou assim: `qemu-img convert` converteu o DMG do Sonoma sem `dmg2img`; o CPU `Skylake-Client,-hle,-rtm` iniciou o Sonoma; e uma VM do VirtualBox rodando fez o QEMU falhar com `KVM_CREATE_VM ... busy`.

Uma revisão encontrou um problema no `fetch-macOS-v2.py`: no fluxo `-s`, o `verify_image()` chama `os.get_terminal_size()` sem tratar a falta de TTY, e o retorno de `action_download()` é descartado. Por isso, rodando com stdout em pipe, a verificação falha e o processo mesmo assim sai com código 0. Os requisitos estão nas specs; a motivação, em proposal.md.

## Goals / Non-Goals

**Goals:**
- Um binário Go que só invoca ferramentas já presentes no host (qemu, git, python3, pkexec).
- Lógica testável (montagem de argumentos, validação, verificações, chunklist, QMP) separada da UI.
- Nunca destruir um disco, uma mídia ou uma NVRAM existentes por efeito colateral.

**Non-Goals:**
- Reconectar a uma VM iniciada fora do app ou em uma execução anterior.
- Atualizar o clone do OSX-KVM (`git pull`) ou fixar uma revisão dele.
- Reimplementar o protocolo de recovery da Apple em Go; só a verificação do chunklist.
- Particionar o disco ou automatizar o instalador dentro do convidado.
- Empacotamento (.deb, Flatpak, AppImage) e regeneração de initramfs.

## Decisions

1. **Fyne v2 como toolkit.** Widgets em Go puro, binário único, sem runtime web. Operações longas rodam em goroutines e atualizam widgets via `fyne.Do` (Fyne ≥ 2.6). Alternativas descartadas: Wails (webkit2gtk e frontend JS, ou seja, duas stacks) e gotk4 (bindings GTK pesados). Custo: CGO e headers OpenGL/X11 no build.
2. **Reusar o OSX-KVM em tempo de execução em vez de embutir.** O app clona o repositório no workspace e usa `OpenCore.qcow2`, OVMF e `fetch-macOS-v2.py` dali, sem redistribuir binários GPLv3 ou da Apple. Um diretório não vazio só é usado se estiver completo; caso contrário, nada é alterado.
3. **Argumentos do QEMU gerados em Go.** O `OpenCore-Boot.sh` é portado para uma função pura `config → []string`, coberta por testes de tabela. Nas opções do QEMU, caminhos têm a vírgula escapada como `,,`. O OpenCore usa `snapshot=on`. O SSH usa `hostfwd=tcp:127.0.0.1:<porta>-:22`. A topologia é `-smp <vcpus>,sockets=1,cores=<c>,threads=<vcpus/c>`.
4. **Perfil de hardware derivado da versão**, seguindo os comentários do upstream: Penryn até Monterey, Skylake-Client de Ventura em diante, vmxnet3 para High Sierra. Isso tira opções da UI e o risco de combinações inválidas. Só o Sonoma foi validado; as demais versões aparecem como experimentais.
5. **NVRAM persistente por workspace.** No primeiro início, `OVMF_VARS-1920x1080.fd` é copiado para `macos-kvm-gui-vars.fd`, que nunca é sobrescrito. Não há opção de resolução: os VARS do upstream e o OpenCore já vêm em 1920x1080, e trocar de VARS perderia a NVRAM.
6. **Download por `fetch-macOS-v2.py`, verificação em Go.** O script roda com `-s <versão>` dentro de `<workspace>/.macos-kvm-gui-staging/`, e seu código de saída não define sucesso. O Go lê o chunklist (header `CNKL`, assinatura RSA-2048 com a chave pública EFI da Apple, igual ao script), confere o SHA-256 de cada chunk do DMG e rejeita bytes sobrando. O progresso vem da saída dividida por `\r` ou `\n`, com o parse de `NN.N% downloaded`; a etapa é inferida pelas linhas `Saving ...chunklist` e `Saving ...dmg`.
7. **Publicação atômica.** `qemu-img convert -p -f dmg -O raw` grava em staging, no mesmo filesystem do destino. Antes do rename, o marcador de versão antigo é apagado; em seguida um `rename` substitui `BaseSystem.img`. O rename é o ponto de commit: antes dele, qualquer falha preserva a mídia anterior. Depois dele, gravar `.macos-kvm-gui-media` é best-effort, e uma falha ali só deixa a versão como "desconhecida", nunca uma versão errada. O staging é sempre removido. Antes de começar, o app exige 4 GiB livres.
8. **Lock por workspace com `flock`** em `$XDG_RUNTIME_DIR/macos-kvm-gui-<sha256(caminho)[:8]>.lock`. O lock fica **fora** do workspace, porque dentro dele o diretório deixaria de estar vazio e o `git clone` recusaria. Ele é mantido durante qualquer operação que altera o workspace e durante toda a execução da VM; uma segunda instância recebe "workspace em uso". O kernel solta o lock se o app morrer. A configuração é copiada no início de cada operação.
9. **QMP com cliente mínimo próprio.** O socket fica em `os.MkdirTemp` (permissão 0700) dentro de `$XDG_RUNTIME_DIR`, ou do diretório temporário se ele não existir. O cliente tenta conectar até 50 vezes, a cada 100 ms, até o QEMU criar o socket. Depois lê o greeting, envia `qmp_capabilities` e mantém uma única conexão durante a execução. Uma goroutine leitora separa eventos (campo `event`) de respostas (`return`/`error`). Cada comando tem 5 s para escrever e receber a resposta. `BLOCK_IO_ERROR` com `action: "stop"` ou um evento `STOP` mudam o estado para "pausada por erro" (o app nunca pausa a VM por conta própria), e `RESUME` volta para "em execução". Se a conexão QMP cair com o processo vivo, o estado continua "em execução", o desligamento gracioso fica indisponível e a parada forçada continua disponível. `system_powerdown` não altera o estado.
10. **Elevação só para `ignore_msrs`.** Um script constante, sem nada interpolado, roda via `pkexec /bin/sh -c '...'`. Ele grava `/sys/module/kvm/parameters/ignore_msrs` e `/etc/modprobe.d/macos-kvm-gui.conf`, um arquivo próprio. Saída 126 do pkexec significa cancelado; qualquer outro código diferente de zero é erro. Depois, o app relê os dois estados para detectar aplicação parcial. A persistência cobre o kvm carregado como módulo; initramfs fica fora de escopo e o README documenta isso.
11. **Conflito de hypervisor como aviso.** O app procura `VBoxHeadless`, `VirtualBoxVM` e `vmware-vmx` em `/proc/*/comm`, e erro de leitura vira "inconclusivo". O diagnóstico definitivo é o erro do QEMU no start (decisão 9 e a tradução de erros).
12. **Configuração em JSON** em `os.UserConfigDir()/macos-kvm-gui/config.json`, gravada de forma atômica (arquivo temporário e rename).
13. **Layout do código:** `main.go` (Fyne) mais `internal/core` (config, host, media, chunklist, qmp, vm), com testes no próprio pacote. Para testar, `/proc` e `/sys`, o executor de comandos e o socket QMP são injetáveis.

## Risks / Trade-offs

- [O upstream muda nomes de arquivos ou a saída do downloader] → Os caminhos ficam em constantes, a verificação do chunklist independe do script e, se o parse de progresso falhar, a barra fica indeterminada.
- [App derrubado com a VM rodando] → O QEMU é filho do app e recebe SIGTERM via `Pdeathsig`. É uma perda equivalente à parada forçada, mas evita um QEMU órfão segurando o workspace sem lock. Como o `Pdeathsig` dispara quando morre a **thread** que criou o processo, `Start` e `Wait` rodam numa goroutine com `runtime.LockOSThread`, e essa thread vive enquanto o QEMU viver.
- [Versões experimentais falham com o perfil derivado] → Ficam marcadas como experimentais na UI; o preflight verifica AVX2/SSE4.1 de Ventura em diante.
- [qemu-img não suporta a compressão de algum DMG] → A conversão falha de forma segura (mídia anterior preservada) e o erro sugere instalar o `dmg2img`.
- [qcow2 thin esgota o disco do host] → Aviso na criação e estado "pausada por erro" vindo do QMP.
- [Build com CGO complica a distribuição] → Aceito nesta fase; o README lista os pacotes.

## Migration Plan

Não se aplica: projeto novo. Um workspace OSX-KVM já existente (`~/OSX-KVM` com disco e mídia) é reaproveitado, porque os nomes dos arquivos são os do upstream. A NVRAM própria nasce no primeiro início feito pelo app.
