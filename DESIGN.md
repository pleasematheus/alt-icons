# alt-icons — decisões de design

Crate Rust em que um executável Windows troca o próprio ícone de forma permanente.

## Escopo

| Decisão | Escolha |
|---|---|
| Permanência | Permanente. O ícone do `.exe` no Explorer muda e sobrevive a reboot. |
| Plataforma | Windows apenas. |
| Superfície | O arquivo `.exe` no Explorer. Sem janela, sem tray, sem console. |
| Distribuição assumida | Executável solto, em qualquer pasta. |
| Assinatura | Binários não assinados. Reescrever o PE invalidaria Authenticode. |
| Conjunto de ícones | Fechado, declarado em tempo de compilação. |

Não é paridade com iOS/Android — lá o ícone trocado é o do lançador. O que a crate
empresta do iOS é o *modelo*: conjunto fechado declarado no build, troca por nome.
O README precisa abrir dizendo isso.

## Um crate, dois contextos de compilação

- **Runtime** — `alt-icons` fornece a API que altera o executável. Dependência:
  `windows-sys` no Windows.
- **Build script** — o mesmo pacote, com a feature `build`, fornece
  `alt_icons::build::configure`. Dependência opcional: `embed-resource`.

O build script é a única declaração de ícones. Ele assa o `Default` no binário e
gera o enum em `OUT_DIR`. Não há proc macro: o build script roda antes da expansão
de macro e não teria como ler uma lista declarada por macro, então a fonte única
de verdade tem que morar nele.

## API

```rust
// build.rs
fn main() {
    alt_icons::build::configure(&[
        ("Default", "assets/default.ico"),
        ("Dark",    "assets/dark.ico"),
    ]);
}
```

```rust
// main.rs
alt_icons::include_icons!();   // include! do arquivo gerado em OUT_DIR

fn main() -> anyhow::Result<()> {
    alt_icons::init()?;                       // limpeza do .old + recuperação
    alt_icons::set_icon(AppIcon::Dark)?;
    Ok(())
}
```

## Mecanismo

1. Mutex nomeado derivado do path do binário. Segunda instância → `SwapInProgress`.
2. Parse completo do `.ico` em memória, montando todos os buffers, **antes de
   qualquer escrita**. Qualquer inconsistência vira erro tipado sem nada ter sido
   escrito no disco.
3. Grava o binário patcheado num temporário **no mesmo diretório do exe**.
4. Se o path original ainda estiver ocupado pelo processo, renomeia para `.old`.
5. Renomeia o temporário para o path original.
6. `SHChangeNotify(SHCNE_ASSOCCHANGED)`.
7. No próximo start, `init()` apaga o `.old` e conserta troca interrompida.

Os ícones alternativos entram no binário via `include_bytes!`, emitido pelo build
script. O exe carrega um único `RT_GROUP_ICON` de id 1 — o mesmo que o
`embed-resource` produz — reescrito por `UpdateResource` a cada troca.

Path não gravável → `IconError::PathNotWritable`. Sem fallback, sem UAC, sem se
copiar para outro lugar.

Sem validação de tamanhos de ícone. Um `.ico` com uma imagem só é aceito, e o
Explorer escala como quiser.

### Armadilhas que a implementação precisa respeitar

- **O temporário tem que ficar no mesmo diretório do exe.** Em `%TEMP%` pode cair
  em outro volume, e aí `MoveFile` vira copiar-e-apagar: perde a atomicidade que
  é a razão de existir da sequência de dois renames.
- **`PathNotWritable` não pode ser testado abrindo o próprio exe para escrita.**
  Isso falha sempre, por definição, enquanto o processo roda. A sondagem tem que
  ser no diretório que contém o exe — criar e apagar um arquivo temporário lá.
  Isso também garante o mesmo volume de graça.
- **A segunda troca na mesma execução não precisa renomear nada.** Depois da
  primeira troca, o processo está mapeado no `.old`; o arquivo no path original
  não está em uso. Renomear às cegas colide com o `.old` que já existe.
- **O path e o estado precisam de cache durante a execução.** Depois que a imagem
  em execução é renomeada, `GetModuleFileNameW` pode passar a devolver o path
  estacionado em `.old`. Além disso, `LoadLibraryExW`, mesmo com
  `LOAD_LIBRARY_AS_DATAFILE`, pode reutilizar a imagem já carregada e devolver
  recursos antigos. O crate fixa o path obtido na primeira consulta e memoriza o
  nome do ícone depois de cada troca; um novo processo volta a ler o estado do PE.

## Verificado empiricamente nesta máquina

- Um exe em execução não abre para escrita, mas **pode ser renomeado**; depois do
  rename dá para gravar um arquivo novo no path original e o processo original
  segue vivo. O `.old` fica travado até o processo morrer.
- `BeginUpdateResource` / `UpdateResource` / `EndUpdateResource` funcionam num exe
  parado; o arquivo cresce e o binário continua executando.
- Injetei o `OneDrive.ico` (8 imagens, 64×64 a 16×16) num `notepad.exe` copiado,
  como grupo de id 1. `LookupIconIdFromDirectoryEx(32×32)` devolveu o `RT_ICON`
  correto, e `PrivateExtractIcons` — a API que o shell usa — passou a extrair o
  ícone injetado no lugar do original. O grupo de menor id vence: o `#1` derrubou
  o `#2` que o notepad já tinha. Controle com cópia não patcheada confirma que o
  exe não corrompe.
- Um exe recém-compilado pelo cargo **não tem nenhum** `RT_GROUP_ICON`. Patcheá-lo
  funciona mesmo assim — a seção de recursos nasce do zero e o programa continua
  rodando — mas o binário distribuído sairia sem ícone. Por isso o `build.rs`.
- `build.rs` com `embed-resource` compilando `1 ICON "icon.ico"` produz o grupo
  `#1`, exatamente o alvo da troca. Trocar por cima do ícone assado mantém um
  único grupo, resolve para o novo ícone, e o programa continua rodando.
  O `rc.exe` não estava no PATH e funcionou: o `embed-resource` acha o SDK sozinho.

## Cache do Explorer — medido

A troca **não depende** de `SHChangeNotify`. Substituindo o arquivo sem disparar
notificação nenhuma, o ícone que o shell entrega para aquele path passa a ser o
novo, nos dois tamanhos, batendo byte a byte com um binário que nasceu com aquele
ícone assado. Medido com `SHGetFileInfo` em processos recém-criados.

Janelas do Explorer **já abertas** são inconsistentes, e a causa não foi isolada.

Numa das três trocas observadas, a janela aberta ficou com o ícone grande novo e o
pequeno velho — as caches por tamanho dentro do `explorer.exe` invalidam de forma
independente. Nessa rodada, nada recuperou o 16×16: `SHCNE_ASSOCCHANGED`,
`SHCNE_UPDATEITEM`, `SHCNE_UPDATEDIR`, `SHCNE_DELETE` seguido de `SHCNE_CREATE`,
`SHCNE_RENAMEITEM`, `SHCNE_ATTRIBUTES`, mexer no `LastWriteTime` e F5 na janela
foram todos sem efeito. Uma cópia byte a byte do arquivo, criada com outro nome na
mesma pasta e na mesma janela, apareceu com o ícone **novo** enquanto o original
seguia com o velho — ou seja, a associação estava presa por caminho na memória do
`explorer.exe`. Reiniciar o Explorer limpou.

Nas duas trocas seguintes, com o Explorer recém-reiniciado, a mesma janela pegou a
troca nos dois tamanhos — inclusive uma em que o `SHChangeNotify` só saiu 45
segundos depois da substituição. A hipótese de que o aviso precisa ser imediato
não se sustentou.

Consequência para o README: o ícone do arquivo muda de verdade, e qualquer consumidor
novo do shell vê o ícone novo. Uma janela do Explorer que já estava mostrando aquele
arquivo **pode** segurar o ícone antigo, em especial o 16×16; quando isso acontece,
reiniciar o Explorer resolve e nada menos que isso resolve. Manter a chamada de
`SHChangeNotify`, porque não custa nada, sem descrevê-la como o que faz a troca valer.

Não é contornável de dentro do processo: a associação vive na memória do
`explorer.exe`, indexada por caminho, e o caminho é justamente o que a crate não
pode mudar.

O hash do binário muda a cada troca. Antivírus heurístico e reputação de
SmartScreen veem um executável que se reescreve. O `build.rs` reduz o dano ao
garantir que isso só acontece quando o usuário do app pede uma troca, nunca na
primeira execução de um binário recém-baixado.

## Testes

Teste de integração: copia um exe de fixture, troca o ícone, e afirma que os bytes
do ícone extraído batem com a entrada correspondente do `.ico` de origem. Não
afirmar sobre o id do `RT_ICON` — ele vem da escolha de tamanho do
`LookupIconIdFromDirectoryEx` e varia com o arquivo e com as métricas do sistema.
Roda em runner Windows no CI, sem inspeção visual.
