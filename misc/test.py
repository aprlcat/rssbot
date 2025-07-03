import os
import json
import requests
from concurrent.futures import ThreadPoolExecutor, as_completed
import logging

logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(message)s')
logger = logging.getLogger(__name__)

def check_feed(feed_data, filename):
    name = feed_data['name']
    url = feed_data['url']
    
    try:
        response = requests.get(url, timeout=10, headers={'User-Agent': 'Mozilla/5.0 RSS Bot'})
        if response.status_code == 200:
            logger.info(f"OK: {name} ({url})")
            return None
        else:
            logger.error(f"FAILED: {name} - HTTP {response.status_code} ({url}) from {filename}")
            return {
                'name': name,
                'url': url,
                'error': f'HTTP {response.status_code}',
                'file': filename
            }
    except requests.exceptions.RequestException as e:
        logger.error(f"FAILED: {name} - {str(e)} ({url}) from {filename}")
        return {
            'name': name,
            'url': url,
            'error': str(e),
            'file': filename
        }

def main():
    opinionated_dir = '../opinionated'
    failed_feeds = []
    
    if not os.path.exists(opinionated_dir):
        logger.error(f"Directory {opinionated_dir} not found")
        return
    
    all_feeds = []
    
    for filename in os.listdir(opinionated_dir):
        if filename.endswith('.json'):
            filepath = os.path.join(opinionated_dir, filename)
            try:
                with open(filepath, 'r') as f:
                    data = json.load(f)
                    for feed in data['feeds']:
                        all_feeds.append((feed, filename))
                        
                logger.info(f"Loaded {len(data['feeds'])} feeds from {filename}")
            except Exception as e:
                logger.error(f"Error loading {filename}: {e}")
    
    logger.info(f"Testing {len(all_feeds)} feeds total")
    
    with ThreadPoolExecutor(max_workers=20) as executor:
        futures = [executor.submit(check_feed, feed, filename) for feed, filename in all_feeds]
        
        for future in as_completed(futures):
            result = future.result()
            if result:
                failed_feeds.append(result)
    
    logger.info(f"Testing complete: {len(failed_feeds)} failed out of {len(all_feeds)}")
    
    if failed_feeds:
        logger.info("Failed feeds summary:")
        for feed in failed_feeds:
            logger.info(f"  {feed['file']}: {feed['name']} - {feed['error']}")

if __name__ == "__main__":
    main()