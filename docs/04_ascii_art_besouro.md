# 04 — Logo das Engrenagens + Olho do HAL-9000 (Overview)

A logo da coluna de **identidade** do Overview (estilo Hermes-Agent): anéis
concêntricos de **engrenagens mecânicas** com o **olho vermelho do HAL-9000**
incandescente ao centro (`src/ascii.rs`).

- **Somente ASCII** (largura previsível com `unicode-width`).
- Todas as linhas de uma arte têm a **mesma largura** (preenchidas com espaços),
  mantendo o olho central alinhado e a colorização radial simétrica.
- Colorização **multi-span por glifo** (ver `ascii::logo_lines`).

> ⚠️ Ao editar, mantenha todas as linhas de uma arte com a **mesma largura**.

---

## Mapa de cores (por glifo)

| Glifo | Elemento             | Cor                       |
|:-----:|----------------------|---------------------------|
| `#`   | dentes do anel externo | bronze `Rgb(180,140,60)` |
| `=`   | vales do anel externo  | cinza escuro `DarkGray`  |
| `x`   | dentes do cubo interno | âmbar `Rgb(210,170,90)`  |
| `+`   | vales do cubo interno  | ouro `Yellow`            |
| `.`   | halo do olho           | `LightRed`               |
| `o`   | íris do olho           | `Red`                    |
| `O`   | núcleo incandescente   | `Rgb(255,50,50)`         |

O olho do HAL é **sempre vermelho**, independente do tema.

---

## Tamanhos (`LogoSize`)

| Tamanho   | Largura × Altura | Anéis            |
|-----------|:----------------:|------------------|
| `Main`    | ~33 × 17         | externo + interno |
| `Medium`  | ~27 × 13         | externo + interno |
| `Compact` | ~20 × 9          | anel único + olho |

```
              =====
        ==####=====####==
      ====             ====
    ##=     xxxx+++++     =##
  ####   ++++x     ++xxx   ####
 ===   x+++...........x+++   ===
 ===  xxx ..ooooooooo.. +++  ===
#==   ++ ..ooooOOOoooo.. ++   ==#
###   ++ ..oooOOOOOooo.. xx   ###
#==   ++ ..ooooOOOoooo.. ++   ==#
 ===  xxx ..ooooooooo.. +++  ===
 ===   x+++...........x+++   ===
  ####   ++++x     ++xxx   ####
    ##=     xxxx+++++     =##
      ====             ====
        ==####=====####==
              =====
```

---

## Regras de Seleção (`ascii::select` + `overview::pick_size`)

- O tamanho é escolhido pela **largura reservada** à coluna da logo
  (`logo_budget = área − GAP − MIN_INFO`) e pela **altura** disponível,
  degradando `Main → Medium → Compact → sem logo` até caber.
- `MIN_INFO` é **fixo** (não depende dos campos detalhados): assim a largura da
  logo permanece **estável ao alternar o modo detalhado** (`.`) — a logo nunca
  encolhe, apenas as seções da direita revelam linhas extras.
- `config.toml` pode fixar a logo:
  `overview.ascii = "main" | "medium" | "compact" | "none" | "auto"`
  (aliases legados: `A`=main, `B`=compact, `C`=medium).
