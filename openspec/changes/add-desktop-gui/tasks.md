# Tasks

## 1. Setup

- [x] 1.1 Criar `go.mod` (módulo `github.com/jniltinho/macos-kvm-gui`), adicionar `fyne.io/fyne/v2` e verificar que `go build ./...` passa
- [x] 1.2 Adicionar `.gitignore` e `Makefile` (alvos `build`, `test`, `run`) e verificar que `make build` gera o binário

## 2. Configuração (vm-lifecycle)

- [x] 2.1 Implementar `internal/core/config.go` com padrões, expansão de `~`, validação (RAM, topologia, porta, caminho) e Load/Save atômico em JSON; verificar com testes de ida e volta, padrões e cada valor inválido
- [x] 2.2 Implementar o perfil de hardware por versão (CPU, NIC, experimental); verificar com teste de tabela das nove versões

## 3. Verificações do host (host-preflight)

- [x] 3.1 Implementar as verificações (CPU vmx/svm, avx2/sse4_1, `/dev/kvm`, ferramentas, versão do QEMU, `ignore_msrs`, processos conflitantes, inconclusivo) com raízes `/proc` e `/sys` e executor injetáveis; verificar com testes sobre árvores falsas em `t.TempDir()`
- [x] 3.2 Implementar a correção de `ignore_msrs` via `pkexec` com script constante, tradução dos códigos de saída e reverificação; verificar com executor simulado (sucesso, 126, erro, parcial)

## 4. Mídia e discos (macos-install-media)

- [x] 4.1 Implementar o lock por workspace com `flock` fora do workspace; verificar com teste de duas aquisições concorrentes e de que o workspace vazio continua vazio
- [x] 4.2 Implementar a classificação do workspace e o clone raso; verificar a classificação (vazio, pronto, incompleto) com teste
- [x] 4.3 Implementar a verificação do chunklist (header, assinatura RSA, hashes, sobra de bytes); verificar com o chunklist real do Sonoma e com DMG adulterado/truncado sintético
- [x] 4.4 Implementar o download em staging, o parse de progresso com `\r` e as etapas; verificar o parse com fluxo fragmentado em teste
- [x] 4.5 Implementar a conversão atômica, o registro de versão, a limpeza do staging e a checagem de 4 GiB livres; verificar com teste que uma conversão falha preserva a mídia anterior e que um marcador antigo não sobrevive à publicação
- [x] 4.6 Implementar a criação do disco (mínimo 40 GiB, sem sobrescrever, aviso de espaço) e a remoção da mídia com os bytes liberados; verificar com `qemu-img` real em `t.TempDir()`

## 5. Ciclo de vida da VM (vm-lifecycle)

- [x] 5.1 Implementar a montagem dos argumentos do QEMU (perfil, `,,`, hostfwd em 127.0.0.1, smp, com e sem mídia, NVRAM própria); verificar com testes de tabela
- [x] 5.2 Implementar o cliente QMP (greeting, capabilities, eventos STOP/RESUME/BLOCK_IO_ERROR por action, powerdown, deadlines, desconexão); verificar com servidor QMP falso em teste
- [x] 5.3 Implementar start, cópia única dos VARS, Pdeathsig com a thread criadora presa (LockOSThread), captura de log, kill e tradução do erro "KVM busy"; verificar com teste de tradução e de não sobrescrever VARS, e com um start real no host de referência

## 6. Interface desktop

- [x] 6.4 Implementar o cálculo de recursos recomendados (threads e MemTotal do host) e o botão na Configuração; verificar com teste de tabela (host de referência e host pequeno)
- [x] 6.5 Mostrar na aba VM a orientação de desempenho de vídeo; verificar com screenshot

- [ ] 6.1 Implementar a janela Fyne com abas Host, Instalação, VM e Log, operações em goroutines e atualização via `fyne.Do`; verificar abrindo o app e executando cada ação no host de referência
- [ ] 6.2 Mostrar os passos manuais da instalação, confirmar a parada forçada e a remoção de mídia, e interceptar o fechamento com a VM rodando; verificar manualmente
- [x] 6.3 Escrever o README (dependências de build/runtime, build, uso, limitações como initramfs e versões experimentais); verificar que os comandos documentados funcionam

## 7. Integração

- [x] 7.1 Rodar `go vet ./...`, `go test -race ./...` e `openspec validate add-desktop-gui --strict` sem erros
