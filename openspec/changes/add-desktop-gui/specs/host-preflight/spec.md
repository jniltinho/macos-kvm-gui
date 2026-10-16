# Spec Delta

## Purpose

Garante que o host Linux tem o necessário para rodar macOS sob KVM e aponta, antes de iniciar a VM, cada problema com sua correção.

## ADDED Requirements

### Requirement: Verificação de pré-requisitos
O sistema SHALL verificar e exibir, cada item com estado OK, aviso, falha ou inconclusivo e uma dica de correção: VT-x/AMD-V na CPU, leitura e escrita em `/dev/kvm`, `qemu-system-x86_64` na versão 8.2.2 ou superior, `qemu-img`, `git`, `python3` e o valor de `ignore_msrs` do módulo kvm.

#### Scenario: Host pronto
- **WHEN** todos os pré-requisitos estão presentes e `ignore_msrs` vale `Y`
- **THEN** todas as verificações aparecem como OK

#### Scenario: Usuário sem acesso ao KVM
- **WHEN** `/dev/kvm` existe mas o usuário não consegue abri-lo para leitura e escrita
- **THEN** a verificação de KVM aparece como falha, com a dica de adicionar o usuário ao grupo `kvm`

#### Scenario: Ferramenta ausente
- **WHEN** `qemu-img` não é encontrado no `PATH`
- **THEN** a verificação correspondente aparece como falha e nomeia o pacote a instalar

#### Scenario: QEMU antigo
- **WHEN** `qemu-system-x86_64 --version` informa versão menor que 8.2.2
- **THEN** a verificação do QEMU aparece como falha e informa a versão mínima

### Requirement: Compatibilidade da CPU com a versão do macOS
O sistema SHALL avisar quando a versão do macOS escolhida for Ventura ou posterior e a CPU do host não tiver as instruções SSE4.1 e AVX2.

#### Scenario: CPU sem AVX2
- **WHEN** a versão escolhida é Sonoma e as flags da CPU não incluem `avx2`
- **THEN** a verificação de compatibilidade aparece como falha e indica uma versão até Monterey

#### Scenario: Versão antiga
- **WHEN** a versão escolhida é Monterey ou anterior
- **THEN** a verificação de compatibilidade não exige AVX2

### Requirement: Indício de hypervisor conflitante
O sistema SHALL procurar processos de VM do VirtualBox ou do VMware e, se encontrar algum, SHALL mostrar um aviso de conflito potencial, sem encerrar processos nem descarregar módulos.

#### Scenario: VM do VirtualBox ativa
- **WHEN** existe um processo `VBoxHeadless` ou `VirtualBoxVM` em execução
- **THEN** a verificação de conflito aparece como aviso e orienta desligar essa VM antes de iniciar o macOS

#### Scenario: Módulo carregado sem VM ativa
- **WHEN** o módulo `vboxdrv` está carregado mas nenhum processo de VM está rodando
- **THEN** a verificação de conflito aparece como OK

#### Scenario: Processos não inspecionáveis
- **WHEN** a lista de processos não pode ser lida
- **THEN** a verificação de conflito aparece como inconclusiva

### Requirement: Correção assistida de ignore_msrs
O sistema SHALL oferecer uma ação que, com elevação de privilégio pedida ao sistema, ativa `ignore_msrs` no kernel em execução e grava um arquivo próprio em `/etc/modprobe.d/`, sem alterar arquivos existentes e nunca guardando a senha.

#### Scenario: Correção aceita
- **WHEN** o usuário aciona a correção e autoriza a elevação
- **THEN** `/sys/module/kvm/parameters/ignore_msrs` passa a valer `Y`, `/etc/modprobe.d/macos-kvm-gui.conf` contém `options kvm ignore_msrs=1` e as verificações são refeitas

#### Scenario: Elevação cancelada
- **WHEN** o usuário cancela o pedido de elevação
- **THEN** nada é alterado e o sistema informa que a correção foi cancelada

#### Scenario: Falha parcial
- **WHEN** a elevação é autorizada mas só uma das duas alterações foi aplicada
- **THEN** o sistema informa qual alteração falhou, verificando o estado real de cada uma depois da execução

#### Scenario: Erro na elevação
- **WHEN** o mecanismo de elevação retorna um erro que não é cancelamento
- **THEN** o sistema mostra o erro retornado e nada é dado como aplicado sem verificação
