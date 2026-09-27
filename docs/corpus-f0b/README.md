# Corpus pessoal F0b

Roteiros de leitura para gravar ~60 min de ditado real com texto-alvo escrito à mão (ULTRAPLAN §3, F0b). O áudio mede os modelos (WER) e o texto-alvo mede o motor de texto pt-BR (pontuação, números, listas, parágrafos, muletas, comandos).

## Como gravar

1. No Handy, em **Histórico**, mude a retenção de gravações para **Nunca apagar**. O padrão guarda só as 5 últimas e apagaria o começo de cada bloco.
2. Abra `leitura.html` (ou a versão publicada) e grave **um bloco por vez**, de 6 a 8 minutos cada, na ordem que quiser.
3. Para cada fala: leia em silêncio, segure o atalho, fale, solte. Se errar, repita; a última tentativa é a que conta.
4. Confira o andamento a qualquer momento:

   ```sh
   python3 scripts/corpus_f0b.py status
   ```

Legenda dos roteiros: `{vírgula}` é um comando dito em voz alta · `_né_` é uma muleta dita de propósito, que não deve sair no texto · `⏸` é uma pausa de ~2 s.

## Conteúdo

| Bloco | Tema                                       | Microfone | Ambiente         |
| ----- | ------------------------------------------ | --------- | ---------------- |
| 01    | Mensagens e agenda                         | embutido  | silêncio         |
| 02    | Plantão e prontuário (pacientes fictícios) | embutido  | silêncio         |
| 03    | E-mails e textos formais                   | embutido  | silêncio         |
| 04    | Tecnologia e trabalho                      | headset   | silêncio         |
| 05    | Finanças, compras e casa                   | headset   | silêncio         |
| 06    | Comandos de voz                            | headset   | silêncio         |
| 07    | Fala espontânea com muletas                | embutido  | TV ou ventilador |
| 08    | Palavras parecidas e textos longos         | embutido  | silêncio         |

São 131 falas e 24 sondas de silêncio, com cerca de 57 min estimados. O `build` confere as cotas do plano: perguntas ≥ 30, listas ≥ 15, multiparágrafo ≥ 15, números ≥ 20, inglês ≥ 15, homófonos ≥ 15.

## Convenções do texto-alvo

Seguem as decisões registradas no ULTRAPLAN §6:

- **Horas:** `18h30`, `20h`, `7h30`. **Dinheiro:** `R$ 15.000`, com centavos só quando ditos (`R$ 399,90`). Euro: `€ 800`.
- **Muletas** (`é…`, `hã`, `ahn`, `né`, `tipo`) e gaguejos são removidos.
- **Listas:** `1. ` ou `- `, item com inicial maiúscula e sem pontuação final. A colagem rica (HTML) é assunto do render (S11); o alvo guarda a forma em texto.
- **Parágrafos:** linha em branco. Nem toda `⏸` vira parágrafo: as pausas de hesitação no meio da frase ficam no mesmo parágrafo, de propósito.
- **Números:** de zero a dez por extenso quando contam coisas ("três dias", "seis pessoas"); em algarismo com unidade de símbolo (`2 kg`, `40 mg`, `95%`, `37,8 °C`), em horas, datas, dinheiro, identificadores (leito 12, sala 3) e acima de dez. O dia 1 do mês vira `1º`. Ordinais em prosa ficam por extenso ("segundo andar").
- **Siglas** em caixa alta exata (PA, SAMU, NIHSS, CCIH).

## Arquivos

- `fonte/bNN.txt` é a fonte de verdade: edite aqui e rode `python3 scripts/corpus_f0b.py build`.
- `manifest.jsonl` traz uma linha por fala (`id`, `tags`, `leia`, `falado`, `alvo`, `mic`, `ambiente`, `estimativa_s`) e é a entrada do harness da F0a.
- `blocos/bloco-NN.md` são os roteiros em Markdown, com o alvo recolhido.
- `leitura.html` é a página de leitura, uma fala por tela, com o progresso salvo no navegador.

O áudio gravado fica na pasta de dados do Handy (`~/Library/Application Support/com.rafael.handylive/recordings`) e **não entra no git**.
