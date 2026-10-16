# Spec Delta

## Purpose

Prepara tudo o que a VM precisa em disco (workspace OSX-KVM, mídia de recovery verificada e disco virtual) usando o mínimo de espaço e sem destruir dados existentes.

## ADDED Requirements

### Requirement: Preparação do workspace OSX-KVM
O sistema SHALL usar um diretório de workspace configurável e SHALL classificá-lo como inexistente ou vazio, pronto, incompleto ou não relacionado. Só um diretório inexistente ou vazio recebe um clone raso (com submódulos) de `https://github.com/kholia/OSX-KVM`.

#### Scenario: Workspace vazio
- **WHEN** o usuário aciona a preparação com um diretório inexistente ou vazio
- **THEN** o repositório é clonado com profundidade 1 e o workspace passa a pronto

#### Scenario: Workspace pronto
- **WHEN** o diretório contém `OpenCore/OpenCore.qcow2`, `OVMF_CODE_4M.fd`, `OVMF_VARS-1920x1080.fd` e `fetch-macOS-v2.py`
- **THEN** nada é baixado e o workspace aparece como pronto

#### Scenario: Diretório não relacionado ou incompleto
- **WHEN** o diretório não está vazio e falta algum desses arquivos
- **THEN** o sistema não clona nem altera nada e informa quais arquivos faltam

### Requirement: Download verificado da recovery
O sistema SHALL baixar a recovery da versão escolhida e SHALL verificar a assinatura da Apple no chunklist e o SHA-256 de cada chunk do DMG antes de usá-lo, independentemente do código de saída do downloader.

#### Scenario: Download íntegro
- **WHEN** o usuário escolhe Sonoma, aciona o download e a verificação passa
- **THEN** o DMG segue para a conversão

#### Scenario: Imagem adulterada ou truncada
- **WHEN** algum chunk não confere com o chunklist ou a assinatura é inválida
- **THEN** o sistema informa falha de verificação, descarta os arquivos baixados e não converte nada

#### Scenario: Falha de rede
- **WHEN** o download é interrompido
- **THEN** o sistema mostra o erro, descarta os arquivos parciais e permite tentar de novo

### Requirement: Progresso por etapa
O sistema SHALL mostrar a etapa atual (download do chunklist, download da imagem, verificação, conversão) e, quando o tamanho total for conhecido, o percentual. Se não for conhecido, o progresso SHALL aparecer como indeterminado.

#### Scenario: Tamanho conhecido
- **WHEN** o downloader informa o percentual baixado
- **THEN** a barra de progresso reflete esse percentual durante o download da imagem

#### Scenario: Tamanho desconhecido
- **WHEN** o downloader não informa percentual
- **THEN** a etapa aparece com progresso indeterminado

### Requirement: Publicação segura da mídia
O sistema SHALL converter o DMG verificado para imagem raw em área temporária no mesmo sistema de arquivos e SHALL só então substituir `BaseSystem.img`, registrar a versão da mídia e apagar o DMG e o chunklist. Qualquer falha SHALL preservar a `BaseSystem.img` anterior.

#### Scenario: Conversão bem-sucedida
- **WHEN** a conversão termina sem erro
- **THEN** `BaseSystem.img` é a nova mídia, a versão registrada é a escolhida e o DMG e o chunklist foram removidos

#### Scenario: Falha na conversão
- **WHEN** a conversão falha, por exemplo por falta de espaço
- **THEN** a `BaseSystem.img` anterior continua intacta, os temporários são removidos e o erro é exibido

#### Scenario: Pouco espaço livre
- **WHEN** o espaço livre no workspace é menor que 4 GiB antes do download
- **THEN** o sistema recusa iniciar e informa o espaço necessário

### Requirement: Criação do disco virtual
O sistema SHALL criar o disco do macOS como qcow2 thin, com tamanho virtual em GiB (mínimo 40), SHALL recusar sobrescrever um disco existente e SHALL avisar que o espaço não fica reservado.

#### Scenario: Disco novo
- **WHEN** o usuário cria um disco de 80 GiB e nenhum disco existe
- **THEN** existe `mac_hdd_ng.img` em qcow2 com 80 GiB de tamanho virtual e menos de 10 MiB alocados no host

#### Scenario: Disco já existe
- **WHEN** o usuário tenta criar um disco e `mac_hdd_ng.img` já existe
- **THEN** o sistema recusa e não altera o arquivo

#### Scenario: Tamanho insuficiente
- **WHEN** o usuário informa menos de 40 GiB
- **THEN** o sistema recusa e informa o mínimo

#### Scenario: Espaço livre menor que o disco
- **WHEN** o espaço livre no host é menor que o tamanho virtual escolhido
- **THEN** o disco é criado e o sistema avisa que a VM pode parar por falta de espaço

### Requirement: Remoção da mídia de instalação
O sistema SHALL permitir remover `BaseSystem.img` somente com a VM parada e depois de o usuário confirmar que o macOS já inicia pelo disco, e SHALL mostrar os bytes liberados (tamanho alocado do arquivo removido).

#### Scenario: Remoção confirmada
- **WHEN** a VM está parada e o usuário confirma
- **THEN** `BaseSystem.img` é apagado e os bytes liberados são exibidos

#### Scenario: Workspace em uso
- **WHEN** a VM ou uma operação de mídia está em execução, nesta ou em outra instância do app
- **THEN** a remoção da mídia é recusada

### Requirement: Exclusão mútua no workspace
O sistema SHALL impedir que duas operações que alteram o workspace (preparação, download, criação ou remoção de disco ou mídia, VM em execução) rodem ao mesmo tempo no mesmo workspace, inclusive entre instâncias do app.

#### Scenario: Segunda instância
- **WHEN** uma instância está com a VM em execução e outra tenta baixar a mídia no mesmo workspace
- **THEN** a segunda instância recusa e informa que o workspace está em uso

### Requirement: Fronteira da automação
O sistema SHALL deixar claro, na interface, que particionar o disco (Disk Utility, APFS) e instalar o macOS dentro do convidado continuam sendo passos manuais.

#### Scenario: Primeira instalação
- **WHEN** o workspace tem mídia e disco, mas o usuário ainda não confirmou que o macOS inicia pelo disco
- **THEN** a interface mostra os passos manuais da instalação dentro da VM
