# Migração de blocos unsafe

## Regras

- Use uma branch e uma versão de patch por etapa.
- Agrupe somente os blocos que pertencem à mesma operação e dependem entre si.
- Preserve a implementação anterior com `#[deprecated]`.
- Não chame as implementações antigas no fluxo atual.
- Preserve os caminhos Windows em UTF-16 e o nome do mutex entre processos.
- Aceite a realocação inicial dos recursos PE. Verifique que ciclos repetidos não acumulam seções.

## Etapas implementadas

| Versão | Branch | Alteração |
|---|---|---|
| 1.2.1 | `refactor/safe-swap-process-id` | Use `std::process::id()` nos nomes temporários. |
| 1.2.2 | `refactor/safe-cleanup-process-id` | Use `std::process::id()` no auxiliar de limpeza. |
| 1.2.3 | `refactor/safe-last-os-error` | Consulte o último erro com `std::io::Error`. Preserve o código `u32`. |
| 1.2.4 | `refactor/safe-current-exe` | Consulte o caminho com `std::env::current_exe()`. Preserve o cache. |
| 1.2.5 | `refactor/safe-clear-last-error` | Limpe o último erro com a API segura do WinSafe. |
| 1.2.6 | `refactor/safe-process-times` | Consulte o horário de criação com WinSafe. Preserve o formato FILETIME. |
| 1.2.7 | `refactor/safe-cleanup-launch` | Inicie o auxiliar com `win-custody`. Feche os handles por RAII. Não herde handles. |
| 1.2.8 | `refactor/safe-resource-update` | Edite recursos com `editpe`. Grave somente na confirmação da transação. |
| 1.2.9 | `fix/resource-ffi-validation` | Valide handles, tamanhos e identificadores nas leituras nativas. Preserve o Windows como referência nos testes. |

As branches formam uma sequência. Cada etapa inclui as etapas anteriores.
A versão declarada no código não comprova publicação no crates.io.

## Blocos nativos mantidos

Há nove blocos ativos na crate e dois no auxiliar de testes. Esta contagem exclui
as implementações antigas marcadas como `deprecated`.

| Local e operação | Blocos ativos | Motivo |
|---|---:|---|
| `win::rename`: `MoveFileExW` | 1 | Preserve caminhos UTF-16 e a opção de falhar quando o destino existe. `std::fs::rename` substitui o destino. |
| `win::notify_shell`: `SHChangeNotify` | 1 | Preserve a notificação ao Explorer. Não foi confirmado um wrapper seguro equivalente. |
| `win::SwapLock`: criação, fechamento e liberação do mutex | 3 | Preserve `Local\alt-icons-*`, a rejeição de um objeto existente e a posse pela thread atual. As alternativas verificadas alteram esse contrato. |
| `win::schedule_cleanup`: `GetSystemDirectoryW` | 1 | Preserve o caminho UTF-16 retornado pelo sistema. O wrapper verificado converte o resultado para `String`. |
| `resources::read`: carregamento, leitura e liberação de recursos | 3 | Preserve a seleção de idioma feita pelo Windows. O novo leitor aceita somente identificadores tipados e verifica `LoadResource` antes de chamar `LockResource`. |
| `fixtures/demo/tests/win`: leitura e seleção de ícones | 2 | Use o Windows para verificar o resultado produzido por `editpe`. Valide o diretório de ícones antes da chamada nativa. |

Os blocos mantidos não são obsoletos. Não os marque como `deprecated` enquanto
forem necessários ao comportamento atual.

## Validação

Execute os comandos na raiz do workspace:

1. `cargo test --workspace --all-features --locked --offline`
2. `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`
3. `cargo fmt --all -- --check`
4. `cargo check -p alt-icons --all-features --target wasm32-unknown-unknown --locked --offline`

Os testes cobrem a troca real de ícones, a limpeza após encerramento normal e
forçado, múltiplas instâncias, o descarte sem confirmação, outros idiomas e
recursos, entradas truncadas e a estabilidade do tamanho após ciclos repetidos.

O limite de crescimento dos testes depende dos executáveis e ícones de teste.
Não é um limite universal em bytes para qualquer PE. A primeira realocação medida
adicionou 47.104 bytes ao executável de demonstração; os quatro ciclos seguintes
mantiveram o tamanho. Os testes adicionais variam os dados de recursos e verificam
que os ciclos seguintes não superam o maior tamanho do primeiro ciclo.

O exemplo de documentação em `lib.rs` permanece ignorado porque depende dos
ícones gerados pelo build script. Os testes unitários e de integração são executados.

As APIs seguras transferem responsabilidades para suas dependências. Esta migração
não afirma que as dependências não contêm `unsafe`.
