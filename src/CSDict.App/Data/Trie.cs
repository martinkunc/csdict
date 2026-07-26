namespace CSDict.App.Data;

/// <summary>Prefix trie over headwords for one language. Traversal keys on
/// char.ToLowerInvariant so prefix search is Unicode-aware case-insensitive (Czech diacritics
/// included), while each node keeps the distinct original-cased spellings that terminate there,
/// since display and the later exact-match SQL lookup both need the real casing.</summary>
internal sealed class Trie
{
    private sealed class Node
    {
        public Dictionary<char, Node>? Children;
        public List<string>? Words;
    }

    private readonly Node _root = new();

    public void Insert(string word)
    {
        if (word.Length == 0)
        {
            return;
        }

        Node node = _root;
        foreach (char c in word)
        {
            char key = char.ToLowerInvariant(c);
            node.Children ??= [];
            if (!node.Children.TryGetValue(key, out Node? child))
            {
                child = new Node();
                node.Children[key] = child;
            }

            node = child;
        }

        node.Words ??= [];
        if (!node.Words.Contains(word))
        {
            node.Words.Add(word);
        }
    }

    /// <summary>Words starting with `prefix`, alphabetically, capped at `limit`. Collects a wider
    /// pool before sorting/truncating so short/common prefixes still return the alphabetically
    /// first matches rather than an arbitrary DFS-order slice, without scanning unboundedly.</summary>
    public List<string> WordsWithPrefix(string prefix, int limit)
    {
        var results = new List<string>();
        if (prefix.Length == 0)
        {
            return results;
        }

        Node node = _root;
        foreach (char c in prefix)
        {
            char key = char.ToLowerInvariant(c);
            if (node.Children is null || !node.Children.TryGetValue(key, out Node? child))
            {
                return results;
            }

            node = child;
        }

        Collect(node, results, Math.Max(limit * 20, 500));
        results.Sort(StringComparer.InvariantCultureIgnoreCase);
        return results.Count > limit ? results.GetRange(0, limit) : results;
    }

    private static void Collect(Node node, List<string> results, int cap)
    {
        if (results.Count >= cap)
        {
            return;
        }

        if (node.Words is not null)
        {
            results.AddRange(node.Words);
        }

        if (node.Children is null)
        {
            return;
        }

        foreach (Node child in node.Children.Values)
        {
            if (results.Count >= cap)
            {
                return;
            }

            Collect(child, results, cap);
        }
    }
}
