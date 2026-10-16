# Proposal

## Why

Instalar macOS no KVM com o [OSX-KVM](https://github.com/kholia/OSX-KVM) exige vários passos manuais no terminal: clonar o repositório, baixar a recovery com `fetch-macOS-v2.py`, converter o DMG, criar o disco qcow2, ajustar `ignore_msrs`, editar RAM/CPU direto no `OpenCore-Boot.sh` e descobrir sozinho conflitos como uma VM do VirtualBox prendendo o AMD-V (`KVM_CREATE_VM: Device or resource busy`). Um app desktop que conduz esses passos e mostra o estado do host reduz os erros e o tempo de configuração.

## What Changes

- Novo app desktop em Go (binário único) que:
  - verifica pré-requisitos do host (CPU com VT-x/AMD-V, acesso a `/dev/kvm`, `qemu-system-x86_64`, `qemu-img`, `git`, `python3`, `ignore_msrs`, hypervisors conflitantes) e oferece corrigir `ignore_msrs`;
  - prepara o workspace do OSX-KVM (clone raso) e reaproveita OpenCore, OVMF e `fetch-macOS-v2.py` dele;
  - baixa a recovery da versão escolhida do macOS (High Sierra → Tahoe; só o Sonoma está validado, as demais são experimentais), verifica a assinatura da Apple, converte para imagem raw e apaga o DMG para economizar espaço;
  - cria o disco virtual qcow2 thin com tamanho configurável;
  - configura RAM, núcleos, vCPUs e porta SSH e salva em um arquivo de configuração do usuário; o modelo de CPU e a placa de rede são derivados da versão do macOS;
  - inicia, desliga (ACPI) e força parada da VM, mantendo a NVRAM UEFI entre execuções e mostrando o log e os erros do QEMU;
  - remove a mídia de instalação depois da instalação, para liberar espaço.
- A lógica do `OpenCore-Boot.sh` é portada para Go: os argumentos do QEMU são gerados a partir da configuração, sem editar scripts. O SSH encaminhado fica restrito a `127.0.0.1`.
- Particionar o disco e rodar o instalador dentro do convidado continuam manuais; o app só mostra os passos.

## Capabilities

### New Capabilities
- `host-preflight`: verificação dos pré-requisitos do host para KVM/macOS e correção assistida de `ignore_msrs`.
- `macos-install-media`: preparação do workspace OSX-KVM, download e conversão da recovery do macOS, criação e limpeza dos discos.
- `vm-lifecycle`: configuração persistente da VM e controle de iniciar, desligar e forçar parada, com acompanhamento do log.

### Modified Capabilities
<!-- Nenhuma: projeto novo. -->

## Impact

- Novo repositório `macos-kvm-gui` (Go, toolkit Fyne).
- Dependências de runtime: `qemu-system-x86_64`, `qemu-img`, `git`, `python3`, `pkexec` (apenas para corrigir `ignore_msrs`).
- Dependências de build: compilador C e headers OpenGL/X11 (exigidos pelo Fyne).
- O app depende de um clone do OSX-KVM (GPLv3) em tempo de execução; nenhum código dele é embutido no binário.
- Fora de escopo nesta mudança: passthrough de GPU/USB, rede em bridge/tap, libvirt, suporte a macOS/Windows como host.
