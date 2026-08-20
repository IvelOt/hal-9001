# 04 — ASCII Art do Besouro (Overview)

Arts do **Besouro / Scarab** usadas na coluna esquerda do Overview (estilo neofetch).
Todas testadas para alinhamento com `unicode-width` (usar apenas ASCII para largura previsível).
A art é colorida por linha via *color spans* no tema (ver `ui/theme.rs`).

> ⚠️ Ao editar, mantenha todas as linhas com a **mesma largura** (preencher com espaços) para não quebrar o layout de duas colunas.

---

## Art A — "Scarab" (principal, ~24 col)

```
        . ~ ~ ~ .
      /  \\ | /  \\
     |    \\|/    |
      \\   (o)   /
   .--- \\ /-\\ / ---.
  /      \\| |/      \\
 |    _.-'\\ /'-._    |
 |  .'     V     '.  |
  \\/   .-'''-.   \\/
   |   /  _  \\   |
   |  |  (_)  |  |
    \\  \\     /  /
     '. '._.' .'
       '-...-'
       /     \\
      '       '
```

## Art B — "Beetle Compacto" (~16 col, telas estreitas)

```
     , _ ,
    ( o o )
   /'` ' `'\\
   |'''''''|
   |\\     /|
   ( \\___/ )
    '.___.'
    /     \\
```

## Art C — "Scarab Detalhado" (fallback largo, ~30 col)

```
          __/\\__
         `==/\\==`
     ___/  ||  \\___
    /   \\  ||  /   \\
   | /\\  \\ || / /\\ |
   | ||   \\||/   || |
    \\ \\   (**)   / /
     \\ '--/  \\--' /
      '.  |  |  .'
    ____'.|  |.'____
   /     /    \\     \\
  '     |      |     '
        '.    .'
          '..'
```

---

## Regras de Seleção (em `ascii.rs`)

- A art é escolhida por largura disponível da coluna esquerda:
  - `>= 30` → Art C
  - `>= 22` → Art A
  - senão → Art B
- `config.toml` pode fixar uma art (`overview.ascii = "A" | "B" | "C" | "auto"`).
- Colorização: gradiente da paleta do tema aplicado por faixa de linhas (carapaça escura → destaque nas antenas/olhos).
