// YAML parser for .gtest spec files using YamlDotNet.
// Mirrors the Rust build.rs parser in crates/grafeo-spec-tests/build.rs.

using YamlDotNet.RepresentationModel;

namespace SpecRunner;

/// <summary>
/// Parses .gtest YAML files into <see cref="GtestFile"/> structures.
/// Uses YamlDotNet's representation model for reliable handling of
/// block scalars, inline lists, and quoted strings.
/// </summary>
public static class GtestParser
{
    /// <summary>Parse a .gtest file at the given path.</summary>
    public static GtestFile ParseFile(string path)
    {
        var content = File.ReadAllText(path);
        return Parse(content);
    }

    /// <summary>Parse .gtest YAML content.</summary>
    public static GtestFile Parse(string content)
    {
        var yaml = new YamlStream();
        using var reader = new StringReader(content);
        yaml.Load(reader);

        if (yaml.Documents.Count == 0)
            return new GtestFile();

        var root = (YamlMappingNode)yaml.Documents[0].RootNode;
        var result = new GtestFile();

        if (root.Children.TryGetValue(new YamlScalarNode("meta"), out var metaNode))
            result.Meta = ParseMeta((YamlMappingNode)metaNode);

        if (root.Children.TryGetValue(new YamlScalarNode("tests"), out var testsNode))
            result.Tests = ParseTests((YamlSequenceNode)testsNode);

        return result;
    }

    private static Meta ParseMeta(YamlMappingNode node)
    {
        var meta = new Meta();

        if (TryGetScalar(node, "language", out var language))
            meta.Language = language;
        if (TryGetScalar(node, "model", out var model))
            meta.Model = model;
        if (TryGetScalar(node, "section", out var section))
            meta.Section = section;
        if (TryGetScalar(node, "title", out var title))
            meta.Title = title;
        if (TryGetScalar(node, "dataset", out var dataset))
            meta.Dataset = dataset;
        if (TryGetStringList(node, "requires", out var requires))
            meta.Requires = requires;
        if (TryGetStringList(node, "tags", out var tags))
            meta.Tags = tags;

        return meta;
    }

    private static List<TestCase> ParseTests(YamlSequenceNode node)
    {
        var tests = new List<TestCase>();
        foreach (var child in node.Children)
        {
            if (child is YamlMappingNode mapping)
                tests.Add(ParseTestCase(mapping));
        }
        return tests;
    }

    private static TestCase ParseTestCase(YamlMappingNode node)
    {
        var tc = new TestCase();

        if (TryGetScalar(node, "name", out var name))
            tc.Name = name;
        if (TryGetScalar(node, "query", out var query))
            tc.Query = query.Trim();
        if (TryGetScalar(node, "skip", out var skip))
            tc.Skip = skip;
        if (TryGetStringList(node, "setup", out var setup))
            tc.Setup = setup;
        if (TryGetStringList(node, "statements", out var statements))
            tc.Statements = statements;
        if (TryGetStringList(node, "tags", out var tags))
            tc.Tags = tags;

        if (TryGetMapping(node, "params", out var paramsNode))
        {
            foreach (var kvp in paramsNode.Children)
            {
                var key = ((YamlScalarNode)kvp.Key).Value ?? "";
                var value = kvp.Value is YamlScalarNode scalar ? scalar.Value ?? "" : "";
                tc.Params[key] = value;
            }
        }

        if (TryGetMapping(node, "variants", out var variantsNode))
        {
            foreach (var kvp in variantsNode.Children)
            {
                var key = ((YamlScalarNode)kvp.Key).Value ?? "";
                var value = kvp.Value is YamlScalarNode scalar ? (scalar.Value ?? "").Trim() : "";
                tc.Variants[key] = value;
            }
        }

        if (TryGetMapping(node, "expect", out var expectNode))
            tc.Expect = ParseExpect(expectNode);

        return tc;
    }

    private static Expect ParseExpect(YamlMappingNode node)
    {
        var expect = new Expect();

        if (TryGetScalar(node, "ordered", out var ordered))
            expect.Ordered = ordered == "true" || ordered == "True";
        if (TryGetScalar(node, "empty", out var empty))
            expect.Empty = empty == "true" || empty == "True";
        if (TryGetScalar(node, "count", out var count) && int.TryParse(count, out var countVal))
            expect.Count = countVal;
        if (TryGetScalar(node, "error", out var error))
            expect.Error = error;
        if (TryGetScalar(node, "hash", out var hash))
            expect.Hash = hash;
        if (TryGetScalar(node, "precision", out var precision) && int.TryParse(precision, out var precVal))
            expect.Precision = precVal;
        if (TryGetStringList(node, "columns", out var columns))
            expect.Columns = columns;

        if (node.Children.TryGetValue(new YamlScalarNode("rows"), out var rowsNode) &&
            rowsNode is YamlSequenceNode rowsSeq)
        {
            expect.Rows = ParseRows(rowsSeq);
        }

        return expect;
    }

    private static List<List<string>> ParseRows(YamlSequenceNode node)
    {
        var rows = new List<List<string>>();
        foreach (var child in node.Children)
        {
            if (child is YamlSequenceNode rowSeq)
            {
                var row = new List<string>();
                foreach (var cell in rowSeq.Children)
                {
                    row.Add(ValueToString(cell));
                }
                rows.Add(row);
            }
            else
            {
                // Single-column shorthand
                rows.Add([ValueToString(child)]);
            }
        }
        return rows;
    }

    /// <summary>
    /// Convert a YAML node to its canonical string representation.
    /// Matches the Rust value_to_string in grafeo-spec-tests/src/lib.rs.
    /// </summary>
    internal static string ValueToString(YamlNode node)
    {
        if (node is YamlScalarNode scalar)
        {
            var value = scalar.Value;

            // null
            if (value is null || value == "null" || value == "~")
                return "null";

            // boolean
            if (value == "true" || value == "True" || value == "TRUE")
                return "true";
            if (value == "false" || value == "False" || value == "FALSE")
                return "false";

            // Try integer first
            if (long.TryParse(value, out _))
                return value;

            // Try float
            if (double.TryParse(value, System.Globalization.NumberStyles.Float,
                System.Globalization.CultureInfo.InvariantCulture, out var d))
            {
                if (double.IsNaN(d)) return "NaN";
                if (double.IsPositiveInfinity(d)) return "Infinity";
                if (double.IsNegativeInfinity(d)) return "-Infinity";
                // Rust's Display for f64 drops ".0" for whole numbers
                if (d == Math.Floor(d) && Math.Abs(d) < (1L << 53))
                    return ((long)d).ToString();
                return d.ToString(System.Globalization.CultureInfo.InvariantCulture);
            }

            return value;
        }

        if (node is YamlSequenceNode seq)
        {
            var inner = string.Join(", ", seq.Children.Select(ValueToString));
            return $"[{inner}]";
        }

        if (node is YamlMappingNode map)
        {
            var entries = map.Children
                .Select(kvp =>
                {
                    var key = ((YamlScalarNode)kvp.Key).Value ?? "";
                    var val = ValueToString(kvp.Value);
                    return $"{key}: {val}";
                })
                .OrderBy(e => e)
                .ToList();
            return "{" + string.Join(", ", entries) + "}";
        }

        return node.ToString() ?? "null";
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    private static bool TryGetScalar(YamlMappingNode node, string key, out string value)
    {
        value = "";
        if (!node.Children.TryGetValue(new YamlScalarNode(key), out var child))
            return false;
        if (child is YamlScalarNode scalar)
        {
            value = scalar.Value ?? "";
            return true;
        }
        return false;
    }

    private static bool TryGetMapping(YamlMappingNode node, string key, out YamlMappingNode mapping)
    {
        mapping = null!;
        if (!node.Children.TryGetValue(new YamlScalarNode(key), out var child))
            return false;
        if (child is YamlMappingNode map)
        {
            mapping = map;
            return true;
        }
        return false;
    }

    private static bool TryGetStringList(YamlMappingNode node, string key, out List<string> list)
    {
        list = [];
        if (!node.Children.TryGetValue(new YamlScalarNode(key), out var child))
            return false;

        if (child is YamlSequenceNode seq)
        {
            list = seq.Children
                .Select(c => c is YamlScalarNode s ? s.Value ?? "" : "")
                .ToList();
            return true;
        }

        if (child is YamlScalarNode scalar && scalar.Value is not null)
        {
            list = [scalar.Value];
            return true;
        }

        return false;
    }
}
