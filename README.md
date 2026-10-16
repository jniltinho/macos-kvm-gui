# macos-kvm-gui

Interface desktop (Go + [Fyne](https://fyne.io)) para instalar e rodar macOS no KVM usando o [OSX-KVM](https://github.com/kholia/OSX-KVM), sem editar scripts.

O app:

- verifica o host (VT-x/AMD-V, `/dev/kvm`, QEMU ≥ 8.2.2, `ignore_msrs`, VMs de VirtualBox/VMware rodando) e corrige o `ignore_msrs` via `pkexec`;
- clona o OSX-KVM (clone raso) no workspace;
- baixa a recovery do macOS, **verifica a assinatura da Apple e o SHA-256 de cada chunk**, converte para imagem raw e apaga o DMG;
- cria o disco qcow2 thin;
- inicia a VM, desliga por ACPI (QMP) ou força a parada, mantendo a NVRAM UEFI entre execuções;
- remove a mídia de instalação depois da instalação, para liberar espaço;
- sugere CPU e RAM para o host e mostra os ajustes de desempenho de vídeo.

Particionar o disco e rodar o instalador **dentro** do macOS continuam manuais; a aba Instalação mostra os passos.

## Dependências

Runtime (Ubuntu/Debian):

```sh
sudo apt install qemu-system-x86 qemu-utils git python3 pkexec
```

Build (Fyne usa CGO, OpenGL, X11 e Wayland):

```sh
sudo apt install gcc pkg-config libgl1-mesa-dev xorg-dev libwayland-dev libxkbcommon-dev wayland-protocols
```

Go 1.27 ou superior (veja `go.mod`).

## Build e uso

```sh
make build   # gera ./macos-kvm-gui
make run
```

Fluxo na interface: **Host** (tudo verde) → **Configuração** (workspace, versão, RAM, CPUs, porta SSH) → **Instalação** (Preparar workspace → Baixar recovery → Criar disco) → **VM** (Iniciar).

Um workspace OSX-KVM já existente (por exemplo `~/OSX-KVM` com `BaseSystem.img` e `mac_hdd_ng.img`) é reaproveitado como está.

O SSH do convidado fica em `127.0.0.1:<porta>` (padrão 2222): `ssh -p 2222 usuario@127.0.0.1`, depois de habilitar o Remote Login no macOS.

## Testes

```sh
make test               # unitários, com -race
make test-integration   # sobe um QEMU real num workspace temporário (exige KVM e ~/OSX-KVM)
```

## Arquivos

| Onde | O quê |
|---|---|
| `~/.config/macos-kvm-gui/config.json` | configuração do app |
| `<workspace>/macos-kvm-gui-vars.fd` | NVRAM UEFI da VM (criada no primeiro início, nunca sobrescrita) |
| `<workspace>/.macos-kvm-gui-media` | versão da `BaseSystem.img` baixada pelo app |
| `$XDG_RUNTIME_DIR/macos-kvm-gui-*.lock` | lock do workspace (uma operação ou VM por vez, inclusive entre instâncias) |
| `/etc/modprobe.d/macos-kvm-gui.conf` | `ignore_msrs` persistente (criado pela correção na aba Host) |

## Desempenho de vídeo

A VM não tem aceleração gráfica: o macOS não tem driver para o vídeo virtual nem para GPUs NVIDIA, então a tela é desenhada pela CPU. O botão **Recomendado para este host** (aba Configuração) usa metade das threads do host (até 16 vCPUs) e um quarto da RAM (até 16 GiB). No macOS, ative *Reduce motion* e *Reduce transparency* (Accessibility › Display) e desligue as animações do Dock. Aceleração de verdade só com uma GPU AMD suportada pelo macOS em passthrough VFIO.

## Limitações

- Só o **Sonoma** foi validado de ponta a ponta. As demais versões aparecem como *experimental*. O modelo de CPU e a placa de rede são escolhidos pela versão (Penryn até Monterey, Skylake-Client de Ventura em diante, vmxnet3 no High Sierra).
- A persistência de `ignore_msrs` vale quando o `kvm` é carregado como módulo. Se ele estiver no initramfs, rode `sudo update-initramfs -u`.
- VirtualBox e VMware não podem estar com VMs rodando ao mesmo tempo que o KVM.
- Fechar o app com a VM rodando é bloqueado. Se o app morrer, o QEMU recebe SIGTERM (como uma parada forçada).
- Sem passthrough de GPU/USB, rede bridge/tap ou libvirt.

## Desenvolvimento

A proposta e as specs estão em [`openspec/changes/add-desktop-gui`](openspec/changes/add-desktop-gui) (OpenSpec).

O OSX-KVM é GPLv3 e é usado em tempo de execução: nada dele é embutido no binário.
