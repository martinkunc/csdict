using System.Buffers.Binary;
using System.IO.Compression;
using System.Net;
using System.Text;
using System.Text.RegularExpressions;

namespace CSDict.App.Scraping;

/// <summary>C# port of dicts/common/stardict.py: a minimal StarDict (.ifo/.idx/.dict) reader, plus
/// a tiny HTML tree builder for WikDict's "h" (html) entry format. Only implements what WikDict's
/// own exports actually use - sametypesequence=h, plain or dictzip(=gzip)-compressed .dict
/// payloads - and is deliberately tolerant of unbalanced/void tags, since WikDict's generator
/// nests sub-senses arbitrarily deep and doesn't always close tags cleanly.</summary>
internal static class StarDict
{
    private static readonly HashSet<string> VoidTags = new(StringComparer.OrdinalIgnoreCase) { "br", "img", "hr", "meta", "link" };

    private static readonly Regex AttrRegex = new(
        """([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))""",
        RegexOptions.Compiled);

    public static Dictionary<string, string> ReadIfo(string path)
    {
        var meta = new Dictionary<string, string>();
        foreach (string rawLine in File.ReadLines(path, Encoding.UTF8))
        {
            string line = rawLine.TrimEnd('\r', '\n');
            int eq = line.IndexOf('=');
            if (eq < 0 || line.StartsWith("StarDict", StringComparison.Ordinal))
            {
                continue;
            }

            meta[line[..eq].Trim()] = line[(eq + 1)..].Trim();
        }

        return meta;
    }

    public readonly record struct IdxRecord(string Word, long Offset, long Size);

    /// <summary>Returns (word, offset, size) for every record, in file order. Assumes
    /// sametypesequence is set in the .ifo (true for WikDict's exports), i.e. no per-record type
    /// byte before offset/size.</summary>
    public static List<IdxRecord> ReadIdx(string path)
    {
        byte[] data = File.ReadAllBytes(path);
        var records = new List<IdxRecord>();
        int i = 0;
        int n = data.Length;
        while (i < n)
        {
            int j = Array.IndexOf(data, (byte)0, i);
            string word = Encoding.UTF8.GetString(data, i, j - i);
            long offset = BinaryPrimitives.ReadUInt32BigEndian(data.AsSpan(j + 1, 4));
            long size = BinaryPrimitives.ReadUInt32BigEndian(data.AsSpan(j + 5, 4));
            records.Add(new IdxRecord(word, offset, size));
            i = j + 9;
        }

        return records;
    }

    public static byte[] ReadDictBlob(string path)
    {
        if (path.EndsWith(".dz", StringComparison.OrdinalIgnoreCase))
        {
            using FileStream fileStream = File.OpenRead(path);
            using var gzip = new GZipStream(fileStream, CompressionMode.Decompress);
            using var buffer = new MemoryStream();
            gzip.CopyTo(buffer);
            return buffer.ToArray();
        }

        return File.ReadAllBytes(path);
    }

    public sealed class Node(string tag)
    {
        public string Tag { get; } = tag;
        public Dictionary<string, string> Attrs { get; } = new();

        /// <summary>Each child is either a string (text) or a Node.</summary>
        public List<object> Children { get; } = [];

        public string Text() => string.Concat(Children.Select(c => c is string s ? s : ((Node)c).Text())).Trim();

        public IEnumerable<Node> FindAll(string tag, IReadOnlyDictionary<string, string>? attrFilter = null)
        {
            foreach (object child in Children)
            {
                if (child is not Node node)
                {
                    continue;
                }

                bool matches = node.Tag == tag
                    && (attrFilter is null || attrFilter.All(kv => node.Attrs.TryGetValue(kv.Key, out string? v) && v == kv.Value));
                if (matches)
                {
                    yield return node;
                }

                foreach (Node nested in node.FindAll(tag, attrFilter))
                {
                    yield return nested;
                }
            }
        }

        /// <summary>A &lt;div&gt; containing only text (no child elements) - WikDict's marker for
        /// "this is a translation".</summary>
        public bool IsLeafDiv() => Tag == "div" && Children.All(c => c is string);
    }

    public static Node ParseHtmlFragment(string html)
    {
        var root = new Node("root");
        var stack = new List<Node> { root };

        int pos = 0;
        while (pos < html.Length)
        {
            int lt = html.IndexOf('<', pos);
            if (lt < 0)
            {
                AppendText(stack, html[pos..]);
                break;
            }

            if (lt > pos)
            {
                AppendText(stack, html[pos..lt]);
            }

            int gt = html.IndexOf('>', lt);
            if (gt < 0)
            {
                AppendText(stack, html[lt..]);
                break;
            }

            string tagContent = html[(lt + 1)..gt];
            pos = gt + 1;

            if (tagContent.Length == 0 || tagContent[0] is '!' or '?')
            {
                continue;
            }

            if (tagContent[0] == '/')
            {
                string endTag = tagContent[1..].Trim().ToLowerInvariant();
                for (int i = stack.Count - 1; i >= 1; i--)
                {
                    if (stack[i].Tag == endTag)
                    {
                        stack.RemoveRange(i, stack.Count - i);
                        break;
                    }
                }

                continue;
            }

            bool selfClosing = tagContent.TrimEnd().EndsWith('/');
            string body = selfClosing ? tagContent[..tagContent.TrimEnd().LastIndexOf('/')] : tagContent;
            (string tag, Dictionary<string, string> attrs) = ParseStartTag(body);
            if (tag.Length == 0)
            {
                continue;
            }

            var node = new Node(tag);
            foreach (KeyValuePair<string, string> kv in attrs)
            {
                node.Attrs[kv.Key] = kv.Value;
            }

            stack[^1].Children.Add(node);

            if (!selfClosing && !VoidTags.Contains(tag))
            {
                stack.Add(node);
            }
        }

        return root;
    }

    private static void AppendText(List<Node> stack, string raw)
    {
        if (raw.Length == 0)
        {
            return;
        }

        string text = WebUtility.HtmlDecode(raw);
        if (text.Length > 0)
        {
            stack[^1].Children.Add(text);
        }
    }

    private static (string Tag, Dictionary<string, string> Attrs) ParseStartTag(string body)
    {
        body = body.Trim();
        int i = 0;
        while (i < body.Length && !char.IsWhiteSpace(body[i]))
        {
            i++;
        }

        string tag = body[..i].ToLowerInvariant();
        var attrs = new Dictionary<string, string>();
        foreach (Match m in AttrRegex.Matches(body[i..]))
        {
            string key = m.Groups[1].Value.ToLowerInvariant();
            string value = m.Groups[2].Success ? m.Groups[2].Value
                : m.Groups[3].Success ? m.Groups[3].Value
                : m.Groups[4].Value;
            attrs[key] = WebUtility.HtmlDecode(value);
        }

        return (tag, attrs);
    }

    /// <summary>Flattens one WikDict record into (pos, ipa, translations, gloss). WikDict/Wiktionary
    /// markup nests sub-senses arbitrarily deep, with translations sometimes given as a parallel
    /// &lt;ol&gt; positionally matching a preceding sub-sense &lt;ol&gt; rather than nested inside
    /// it. Rather than reconstruct that alignment, every leaf &lt;div&gt; found anywhere in the
    /// record becomes one pooled translation, and every other bit of text becomes part of one
    /// pooled gloss. This loses the sub-sense &lt;-&gt; translation mapping but keeps 100% of the
    /// actual words on both sides.</summary>
    public static (string? Pos, string? Ipa, List<string> Translations, string? Gloss) ExtractPosIpaTranslationsGloss(Node root)
    {
        Node? grammarFont = root.FindAll("font", new Dictionary<string, string> { ["class"] = "grammar" }).FirstOrDefault();
        string? pos = grammarFont?.Text() is { Length: > 0 } posText ? posText : null;

        Node? ipaFont = root.FindAll("font", new Dictionary<string, string> { ["color"] = "gray" }).FirstOrDefault();
        string? ipa = ipaFont?.Text() is { Length: > 0 } ipaText ? ipaText : null;

        // The pos-wrapper div is `<div><font class="grammar">POS</font></div>`. Identify it by
        // identity (not structural equality - two different senses can share the same pos text)
        // and skip exactly that one node, plus the leading ipa/font/br clutter before it, then
        // flatten everything else in document order.
        Node? posWrapper = root.FindAll("div").FirstOrDefault(div =>
            div.Children.Count == 1
            && div.Children[0] is Node only
            && only.Tag == "font"
            && only.Attrs.TryGetValue("class", out string? cls)
            && cls == "grammar");

        var translations = new List<string>();
        var glossParts = new List<string>();
        bool seenPosWrapper = false;

        void Walk(Node node)
        {
            foreach (object child in node.Children)
            {
                if (child is string rawText)
                {
                    string text = rawText.Trim().Trim('/', ',');
                    if (text.Length > 0 && seenPosWrapper)
                    {
                        glossParts.Add(text);
                    }

                    continue;
                }

                var childNode = (Node)child;
                if (childNode.Tag == "font")
                {
                    continue;
                }

                if (ReferenceEquals(childNode, posWrapper))
                {
                    seenPosWrapper = true;
                    continue;
                }

                if (!seenPosWrapper)
                {
                    Walk(childNode);
                    continue;
                }

                if (childNode.IsLeafDiv())
                {
                    string text = childNode.Text();
                    if (text.Length > 0 && !translations.Contains(text))
                    {
                        translations.Add(text);
                    }
                }
                else
                {
                    Walk(childNode);
                }
            }
        }

        Walk(root);
        string? gloss = glossParts.Count > 0 ? string.Join("; ", glossParts) : null;
        return (pos, ipa, translations, gloss);
    }
}
