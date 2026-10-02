namespace EveDogmaK.Data;

public static class DatasetCache
{
    public static Dataset Load(string path) => DatasetLoader.LoadPath(path);
}
