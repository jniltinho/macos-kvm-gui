# Spec Delta

## Purpose

Permite configurar a VM do macOS e controlá-la (iniciar, desligar, forçar parada) sem editar scripts, acompanhando o que o QEMU reporta.

## ADDED Requirements

### Requirement: Configuração persistente da VM
O sistema SHALL permitir configurar workspace, versão do macOS, RAM (MiB), núcleos, vCPUs totais e porta SSH do host, e SHALL guardar esses valores entre execuções do app.

#### Scenario: Valores mantidos
- **WHEN** o usuário salva RAM de 8192 MiB e reabre o app
- **THEN** o campo de RAM mostra 8192

#### Scenario: Primeira execução
- **WHEN** não há configuração salva
- **THEN** o app usa os padrões: workspace `~/OSX-KVM`, Sonoma, 8192 MiB, 4 núcleos, 8 vCPUs, porta 2222

### Requirement: Validação da configuração
O sistema SHALL recusar salvar RAM abaixo de 4096 MiB, núcleos ou vCPUs menores que 1, vCPUs que não sejam múltiplo dos núcleos, porta fora de 1024–65535 e workspace que não seja caminho absoluto após expandir `~`.

#### Scenario: Topologia impossível
- **WHEN** o usuário informa 4 núcleos e 6 vCPUs
- **THEN** o sistema recusa e indica o campo de vCPUs

#### Scenario: Porta inválida
- **WHEN** o usuário informa a porta 80
- **THEN** o sistema recusa e indica o intervalo permitido

### Requirement: Perfil de hardware por versão
O sistema SHALL escolher o modelo de CPU e a placa de rede a partir da versão do macOS: Penryn até Monterey, Skylake-Client de Ventura em diante, e vmxnet3 para High Sierra. Somente as versões validadas SHALL aparecer sem a marca "experimental".

#### Scenario: Sonoma
- **WHEN** a versão é Sonoma
- **THEN** a VM usa CPU Skylake-Client e rede virtio

#### Scenario: High Sierra
- **WHEN** a versão é High Sierra
- **THEN** a VM usa CPU Penryn e rede vmxnet3 e a versão aparece como experimental

### Requirement: Iniciar a VM
O sistema SHALL iniciar o QEMU com KVM, máquina q35, OpenCore em modo snapshot, firmware OVMF com variáveis UEFI persistentes, disco do macOS, rede NAT e SSH encaminhado apenas de `127.0.0.1:<porta>` para a porta 22 do convidado.

#### Scenario: Início com mídia de instalação
- **WHEN** o usuário inicia a VM e `BaseSystem.img` existe
- **THEN** o QEMU sobe com a mídia de instalação conectada e a janela da VM abre

#### Scenario: Início sem mídia de instalação
- **WHEN** o usuário inicia a VM e `BaseSystem.img` não existe
- **THEN** o QEMU sobe apenas com o disco do macOS

#### Scenario: Disco ausente
- **WHEN** o usuário tenta iniciar e `mac_hdd_ng.img` não existe
- **THEN** o sistema não inicia o QEMU e orienta criar o disco

#### Scenario: SSH restrito ao host
- **WHEN** a VM está em execução com a porta 2222
- **THEN** a porta 2222 aceita conexões apenas em `127.0.0.1`

### Requirement: Persistência das variáveis UEFI
O sistema SHALL criar, no primeiro início, uma cópia própria do arquivo de variáveis UEFI a partir de `OVMF_VARS-1920x1080.fd` e SHALL reutilizá-la nos inícios seguintes, sem nunca sobrescrevê-la.

#### Scenario: Segundo início
- **WHEN** a VM já foi iniciada uma vez e é iniciada de novo
- **THEN** o QEMU usa a mesma cópia de variáveis UEFI, com as alterações feitas pelo convidado preservadas

### Requirement: Parada da VM
O sistema SHALL oferecer desligamento gracioso (pedido ACPI de energia ao convidado), que só é dado como concluído quando o QEMU termina, e parada forçada do processo, que pede confirmação avisando sobre perda de dados.

#### Scenario: Desligamento gracioso
- **WHEN** o usuário aciona desligar com a VM em execução
- **THEN** o convidado recebe o pedido e o estado continua "em execução" até o QEMU terminar, sem escalar sozinho para parada forçada

#### Scenario: Parada forçada
- **WHEN** o usuário confirma a parada forçada
- **THEN** o processo QEMU é encerrado e o estado passa a parado

### Requirement: Acompanhamento do QEMU
O sistema SHALL exibir o estado da VM (parada, em execução ou pausada por erro) e a saída do QEMU, e SHALL traduzir em mensagens claras as falhas conhecidas.

#### Scenario: KVM ocupado
- **WHEN** o QEMU sai com `failed to initialize kvm: Device or resource busy`
- **THEN** o estado volta a parado e o sistema informa que outro hypervisor está usando a virtualização

#### Scenario: Erro de disco
- **WHEN** o QEMU pausa o convidado por erro de I/O no disco, por exemplo por falta de espaço no host
- **THEN** o estado passa a "pausada por erro" com a mensagem do erro, mesmo com o processo vivo

#### Scenario: Saída normal
- **WHEN** o convidado desliga e o QEMU termina com código 0
- **THEN** o estado volta a parado sem mensagem de erro

#### Scenario: Fechar o app com a VM rodando
- **WHEN** o usuário fecha a janela com a VM em execução
- **THEN** o app não fecha e pede para desligar a VM antes

### Requirement: Recursos recomendados para o host
O sistema SHALL oferecer uma ação que preenche núcleos, vCPUs e RAM com valores recomendados para o host: vCPUs iguais à metade das threads do host (par, entre 2 e 16), núcleos iguais à metade das vCPUs (mínimo 1) e RAM igual a um quarto da memória total, arredondada para baixo em múltiplos de 1024 MiB, entre 4096 e 16384 MiB. Sem GPU suportada, o vídeo é renderizado pela CPU.

#### Scenario: Host de referência
- **WHEN** o host tem 24 threads e 186 GiB de RAM
- **THEN** os campos passam a 12 vCPUs, 6 núcleos e 16384 MiB, sem salvar até o usuário confirmar

#### Scenario: Host pequeno
- **WHEN** o host tem 4 threads e 8 GiB de RAM
- **THEN** os campos passam a 2 vCPUs, 1 núcleo e 4096 MiB

### Requirement: Orientação de desempenho de vídeo
O sistema SHALL informar que a VM não tem aceleração gráfica (o macOS não tem driver para o vídeo virtual nem para GPUs NVIDIA) e SHALL listar os ajustes no macOS que reduzem o custo de renderização.

#### Scenario: Aba da VM
- **WHEN** o usuário abre a aba da VM
- **THEN** vê os ajustes "Reduce motion", "Reduce transparency" e animações do Dock, e a observação de que só uma GPU AMD suportada em passthrough traz aceleração
