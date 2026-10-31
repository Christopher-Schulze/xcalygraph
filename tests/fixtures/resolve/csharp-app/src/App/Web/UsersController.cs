using App.Store;
using Repo = App.Store.IRepository;

namespace App.Web
{
    [ApiController]
    public class UsersController
    {
        private readonly Repo repository;

        public UsersController(Repo repository)
        {
            this.repository = repository;
        }

        [HttpGet]
        public string Get(int id)
        {
            Row? row = repository.Find(id);
            return row is null ? "" : row.Label();
        }

        public static void Main(string[] args)
        {
            var controller = new UsersController(new MemoryRepository());
            var url = Environment.GetEnvironmentVariable("APP_URL");
            Console.WriteLine(controller.Get(1) + url);
        }
    }
}
