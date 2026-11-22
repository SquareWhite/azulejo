// Scores smart select's command detection on a fictional corpus: the rules
// (the Rust core, exported by `azj-hints-candidates`) against Apple's
// on-device model picking commands from the same candidates with guided
// generation, plus latency (cold and warm). Everything stays on this Mac:
// FoundationModels runs the system model locally.
//
//   swift run -c release HintsModelBench <candidates.json>
//
// Guided generation uses a DynamicGenerationSchema rather than @Generable:
// the macro's plugin ships with Xcode, not the Command Line Tools, and the
// schema is the same constrained decoding either way.

import Foundation
import FoundationModels

struct Screen: Decodable {
    let name: String
    let category: String
    let rows: [String]
    let expected: [String]
    let rules: [String]
    let candidates: [String]
}

struct Score {
    var found = 0, missed = 0, extra = 0
    var precision: Double { found + extra == 0 ? 1 : Double(found) / Double(found + extra) }
    var recall: Double { found + missed == 0 ? 1 : Double(found) / Double(found + missed) }

    mutating func add(expected: [String], got: [String]) {
        var left = got
        for e in expected {
            if let i = left.firstIndex(of: e) { left.remove(at: i); found += 1 } else { missed += 1 }
        }
        extra += left.count
    }

    func line(_ name: String) -> String {
        name.padding(toLength: 14, withPad: " ", startingAt: 0)
            + String(format: " precision %5.1f%%  recall %5.1f%%  (%d found, %d missed, %d extra)",
                     precision * 100, recall * 100, found, missed, extra)
    }
}

let instructions = """
    You read a terminal screen and pick the shell commands on it that a user would want to run: \
    commands typed at a prompt, commands an assistant suggests, commands in code blocks or hints \
    ("use git push"). Never pick prose, program output, log lines, file contents, paths on their \
    own, status bars, progress bars, error messages or code that isn't a shell command. Copy each \
    command exactly from its candidate, without a prompt, bullet, number or surrounding sentence.
    """

/// `{ commands: [{ candidate: Int, command: String }] }`.
func schema() throws -> GenerationSchema {
    let pick = DynamicGenerationSchema(name: "Pick", properties: [
        .init(name: "candidate", description: "The candidate's number", schema: .init(type: Int.self)),
        .init(name: "command", description: "The command, copied exactly from the candidate",
              schema: .init(type: String.self)),
    ])
    let picks = DynamicGenerationSchema(name: "Picks", properties: [
        .init(name: "commands", schema: .init(arrayOf: .init(referenceTo: "Pick"))),
    ])
    return try GenerationSchema(root: picks, dependencies: [pick])
}

func prompt(_ screen: Screen) -> String {
    let rows = screen.rows.joined(separator: "\n")
    let candidates = screen.candidates.enumerated().map { "\($0.offset): \($0.element)" }.joined(separator: "\n")
    return "Screen:\n\(rows)\n\nCandidates:\n\(candidates)\n\nWhich candidates hold commands to run?"
}

/// The model's picks, kept only when the command is text from its candidate.
func picks(_ content: GeneratedContent, _ screen: Screen) -> [String] {
    guard case .structure(let properties, _) = content.kind,
          case .array(let items)? = properties["commands"]?.kind else { return [] }
    var result: [String] = []
    for item in items {
        guard case .structure(let fields, _) = item.kind,
              case .number(let n)? = fields["candidate"]?.kind,
              case .string(let raw)? = fields["command"]?.kind else { continue }
        let index = Int(n), command = raw.trimmingCharacters(in: .whitespaces)
        guard screen.candidates.indices.contains(index), !command.isEmpty,
              screen.candidates[index].contains(command), !result.contains(command) else { continue }
        result.append(command)
    }
    return result
}

func median(_ values: [Double]) -> Double {
    let sorted = values.sorted()
    return sorted.isEmpty ? 0 : sorted[sorted.count / 2]
}

@main
struct Bench {
    static func main() async throws {
        let args = CommandLine.arguments
        guard args.count >= 2 else {
            FileHandle.standardError.write(Data("usage: HintsModelBench <candidates.json> [--verbose]\n".utf8))
            exit(2)
        }
        let verbose = args.contains("--verbose")
        let screens = try JSONDecoder().decode([Screen].self, from: Data(contentsOf: URL(fileURLWithPath: args[1])))
        var rules = Score()
        var byCategory: [String: Score] = [:]
        for screen in screens {
            rules.add(expected: screen.expected, got: screen.rules)
            byCategory[screen.category, default: Score()].add(expected: screen.expected, got: screen.rules)
        }
        print("corpus: \(screens.count) screens, \(screens.reduce(0) { $0 + $1.expected.count }) commands")
        print(rules.line("rules"))
        for (category, score) in byCategory.sorted(by: { $0.key < $1.key }) { print("  " + score.line(category)) }

        let model = SystemLanguageModel.default
        guard case .available = model.availability else {
            print("model: unavailable (\(model.availability)); nothing to compare")
            return
        }
        let schema = try schema()
        let options = GenerationOptions(sampling: .greedy)
        var alone = Score(), filtered = Score(), union = Score()
        var latencies: [Double] = []
        var cold: Double?
        var failures = 0
        for screen in screens {
            let start = Date()
            let session = LanguageModelSession(model: model, instructions: instructions)
            var got: [String] = []
            do {
                let response = try await session.respond(to: prompt(screen), schema: schema, options: options)
                got = picks(response.content, screen)
            } catch {
                failures += 1
                if verbose { print("  \(screen.name): \(error)") }
            }
            let elapsed = Date().timeIntervalSince(start) * 1000
            if cold == nil { cold = elapsed } else { latencies.append(elapsed) }
            alone.add(expected: screen.expected, got: got)
            filtered.add(expected: screen.expected, got: screen.rules.filter(got.contains))
            union.add(expected: screen.expected, got: screen.rules + got.filter { !screen.rules.contains($0) })
            if verbose, Set(got) != Set(screen.expected) {
                print("  \(screen.name): model \(got) expected \(screen.expected)")
            }
        }
        print(alone.line("model"))
        print(filtered.line("rules ∩ model"))
        print(union.line("rules ∪ model"))
        print(String(format: "latency: cold %.0f ms, warm median %.0f ms, warm max %.0f ms; %d errors",
                     cold ?? 0, median(latencies), latencies.max() ?? 0, failures))

        // A full screen: every fixture's rows, 50 at a time, as one prompt.
        let all = screens.flatMap(\.rows)
        let full = Screen(name: "full", category: "full", rows: Array(all.prefix(50)), expected: [], rules: [],
                          candidates: screens.prefix(12).flatMap(\.candidates))
        let session = LanguageModelSession(model: model, instructions: instructions)
        let start = Date()
        _ = try? await session.respond(to: prompt(full), schema: schema, options: options)
        print(String(format: "full screen (50 rows, %d candidates): %.0f ms",
                     full.candidates.count, Date().timeIntervalSince(start) * 1000))
    }
}
